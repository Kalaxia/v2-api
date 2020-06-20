use actix_web::{get, web, HttpResponse};
use actix::prelude::*;
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use std::sync::RwLock;
use std::collections::{HashMap};
use std::time::Duration;
use std::sync::{Arc, Mutex};
use futures::executor::block_on;
use crate::{
    lib::{Result, error::InternalError},
    game::{
        faction::{FactionID},
        fleet::{
            fleet::{Fleet, FleetID, FLEET_TRAVEL_TIME},
        },
        lobby::Lobby,
        player::{PlayerID, Player},
        system::{SystemID, System, FleetArrivalOutcome, generate_systems}
    },
    ws::{ client::ClientSession, protocol},
    AppState,
};

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct GameID(pub Uuid);

#[derive(Serialize, Deserialize, Clone)]
pub struct GameData {
    pub id: GameID,
    pub systems: HashMap<SystemID, System>
}

pub struct Game {
    data: Arc<Mutex<GameData>>,
    state: web::Data<AppState>,
    clients: RwLock<HashMap<PlayerID, actix::Addr<ClientSession>>>,
}

pub const MAP_SIZE: u8 = 10;
pub const TERRITORIAL_DOMINION_RATE: u8 = 60;

impl From<GameID> for Uuid {
    fn from(gid: GameID) -> Self { gid.0 }
}

impl Game {
    fn init(&mut self) {
        let mut data = self.data.lock().expect("Poisoned lock on game data");
        (*data).systems = generate_systems();
        drop(data); 
        block_on(self.assign_systems());
    }

    fn begin(&self) {
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::GameStarted,
            self.data.lock().expect("Poisoned lock on game data").clone()
        ), None);
    }

    fn ws_broadcast(&self, message: protocol::Message, skip_id: Option<PlayerID>) {
        let clients = self.clients.read().expect("Poisoned lock on game clients");
        for (id, client) in clients.iter() {
            if Some(*id) != skip_id {
                client.do_send(message.clone());
            }
        }
    }

    async fn assign_systems(&mut self) {
        let mut placed_per_faction = HashMap::new();
        let mut data = self.data.lock().expect("Poisoned lock on game data");
        let players = Player::find_by_game(data.id, &self.state.db_pool).await;

        for player in players {
            let fid = player.faction.unwrap().clone();
            let i = placed_per_faction.entry(fid).or_insert(0);
            let place = self.find_place(fid, *i, &data.systems);
            *i += 1;

            if let Some(place) = place {
                // legitimate use of unwrap, because we KNOW `place` IS an existing system id
                // if it is Some()
                data.systems.get_mut(&place).unwrap().player = Some(player.id);
            } else {
                // else do something to handle the non-placed player
                // here we put unreachable!() because it is normaly the case.
                unreachable!()
            }
        }
    }

    fn find_place(&self, fid: FactionID, i: u8, systems: &HashMap<SystemID, System>) -> Option<SystemID> {
        // Each faction is associated to a side of the grid
        let coordinates_check: & dyn Fn(u8, u8, u8) -> bool = match fid {
            FactionID(1) => &|i, x, y| x > 0 || y < i,
            FactionID(2) => &|i, x, y| x < MAP_SIZE - 1 || y < i,
            FactionID(3) => &|i, x, y| y > 0 || x < i,
            FactionID(4) => &|i, x, y| y < MAP_SIZE - 1 || x < i,
            _ => unimplemented!() // better than "None" because normaly this function is total
        };
        for (sid, system) in systems {
            if coordinates_check(i, system.coordinates.x, system.coordinates.y) || system.player != None {
                continue;
            }
            return Some(sid.clone());
        }
        None
    }

    async fn produce_income(&mut self) {
        let data = self.data.lock().expect("Poisoned lock on game data");
        let mut players: HashMap<PlayerID, Player> = Player::find_by_game(data.id, &self.state.db_pool).await
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect();
        let mut players_income = HashMap::new();

        // Add money to each player based on the number of
        // currently, the income is `some_player.income = some_player.number_of_systems_owned * 15`
        data.systems
            .values() // for each system
            .flat_map(|system| system.player) // with a player in it
            .for_each(|player| *players_income.entry(player).or_insert(0) += 15); // update the player's income

        // Notify the player for wallet update
        #[derive(Serialize, Clone)]
        struct PlayerIncome {
            income: usize
        }
        for (pid, income) in players_income {
            players.get_mut(&pid).map(|p| {
                p.wallet += income;
                // p.websocket.as_ref().map(|ws| {
                //     ws.do_send(protocol::Message::new(
                //         protocol::Action::PlayerIncome,
                //         PlayerIncome{ income }
                //     ));
                // });
            });
        }
    }

    async fn process_fleet_arrival(&mut self, fleet_id: FleetID, system_id: SystemID) -> Result<()> {
        let mut data = self.data.lock().expect("Poisoned lock on game data");
        let fleet = {
            let system = data.systems.get_mut(&system_id).ok_or(InternalError::SystemUnknown)?;
            let f = system.fleets.get_mut(&fleet_id).ok_or(InternalError::FleetUnknown)?.clone();
            system.fleets.remove(&fleet_id.clone());
            f
        };
        let destination_system = data.systems.get_mut(&fleet.destination_system.unwrap()).ok_or(InternalError::SystemUnknown)?;

        let player = Player::find(fleet.player, &self.state.db_pool).await.ok_or(InternalError::PlayerUnknown)?;

        let system_owner = {
            match destination_system.player {
                Some(owner_id) => Some(Player::find(owner_id, &self.state.db_pool).await.ok_or(InternalError::PlayerUnknown)?),
                None => None,
            }
        };
        let result = destination_system.resolve_fleet_arrival(fleet, &player, system_owner);
        drop(data);
        self.ws_broadcast(result.clone().into(), None);
        if let FleetArrivalOutcome::Conquerred{ fleet: _fleet, system: _system } = result {
            self.check_victory().await;
        }
        Ok(())
    }

    async fn check_victory(&mut self) {
        let data = self.data.lock().expect("Poisoned lock on game data");
        let players: HashMap<PlayerID, Player> = Player::find_by_game(data.id, &self.state.db_pool).await
            .into_iter()
            .map(|p| (p.id, p))
            .collect();
        let mut territories = HashMap::new();
        let total_territories = data.systems.len() as f64;
        let nb_territories_to_win = (total_territories * (TERRITORIAL_DOMINION_RATE as f64) / 100.0).ceil() as u8;

        data.systems
            .values() // for each system
            .flat_map(|system| system.player)
            .map(|pid| players.get(&pid).expect("Player not found")) // with a player in it
            .for_each(|player| *territories.entry(player.faction.unwrap().clone()).or_insert(0) += 1); // update the player's income
        drop(players);
        drop(data);

        for (fid, nb_systems) in territories.iter() {
            if *nb_systems >= nb_territories_to_win {
                #[derive(Serialize, Clone)]
                struct VictoryData {
                    victorious_faction: FactionID,
                    scores: HashMap<FactionID, u8>
                }
                self.ws_broadcast(protocol::Message::new(
                    protocol::Action::Victory,
                    VictoryData{
                        victorious_faction: *fid,
                        scores: territories,
                    }
                ), None);
                return;
            }
        }
    }

    pub async fn remove_player(&self, pid: PlayerID) -> Result<()> {
        let mut player = Player::find(pid, &self.state.db_pool).await.ok_or(InternalError::PlayerUnknown)?;
        player.is_connected = false;
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::PlayerLeft,
            pid.clone()
        ), Some(pid));
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        let clients = self.clients.read().expect("Poisoned lock on game players");
        
        clients.len() == 0
    }
}

impl Actor for Game {
    type Context = Context<Self>;
    
    fn started(&mut self, ctx: &mut Context<Self>) {
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::LobbyLaunched,
            self.data.lock().expect("Poisoned lock on game data").clone()
        ), None);
        ctx.run_later(Duration::new(1, 0), |this, _| this.init());
        ctx.run_later(Duration::new(5, 0), |this, _| this.begin());
        ctx.run_interval(Duration::new(5, 0), move |this, _| {
            block_on(this.produce_income());
        });
    }
 
    fn stopped(&mut self, _ctx: &mut Context<Self>) {
        println!("Game is stopped");
    }
}

#[derive(Serialize, Clone)]
pub struct GameDataMessage{}
#[derive(actix::Message, Serialize, Clone)]
#[rtype(result="bool")]
pub struct GameRemovePlayerMessage(pub PlayerID);

#[derive(actix::Message)]
#[rtype(result="()")]
pub struct GameBroadcastMessage {
    pub message: protocol::Message,
    pub skip_id: Option<PlayerID>
}

#[derive(actix::Message)]
#[rtype(result="()")]
pub struct GameFleetTravelMessage{
    pub fleet: Fleet
}

impl actix::Message for GameDataMessage {
    type Result = Arc<Mutex<GameData>>;
}

impl Handler<GameDataMessage> for Game {
    type Result = Arc<Mutex<GameData>>;

    fn handle(&mut self, _msg: GameDataMessage, _ctx: &mut Self::Context) -> Self::Result {
        self.data.clone()
    }
}

impl Handler<GameRemovePlayerMessage> for Game {
    type Result = bool;

    fn handle(&mut self, GameRemovePlayerMessage(pid): GameRemovePlayerMessage, ctx: &mut Self::Context) -> Self::Result {
        block_on(self.remove_player(pid));
        if self.is_empty() {
            ctx.stop();
            ctx.terminate();
            return true;
        }
        false
    }
}

impl Handler<GameBroadcastMessage> for Game {
    type Result = ();

    fn handle(&mut self, msg: GameBroadcastMessage, _ctx: &mut Self::Context) -> Self::Result {
        self.ws_broadcast(msg.message, msg.skip_id);
    }
}

impl Handler<GameFleetTravelMessage> for Game {
    type Result = ();

    fn handle(&mut self, msg: GameFleetTravelMessage, ctx: &mut Self::Context) -> Self::Result {
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::FleetSailed,
            msg.fleet.clone()
        ), Some(msg.fleet.player));
        ctx.run_later(Duration::new(FLEET_TRAVEL_TIME.into(), 0), move |this, _| {
            block_on(this.process_fleet_arrival(msg.fleet.id.clone(), msg.fleet.system.clone()));
        });
        ()
    }
}

pub async fn create_game(lobby: &Lobby, state: web::Data<AppState>) -> (GameID, Addr<Game>) {
    let id = GameID(Uuid::new_v4());
    let mut game_players = HashMap::new();
    let players = Player::find_by_lobby(lobby.id, &state.db_pool).await;
    
    for mut p in players {
        p.lobby = None;
        p.game = Some(id.clone());
        game_players.insert(p.id, p.clone());
    }
    let game = Game{
        state: state.clone(),
        data: Arc::new(Mutex::new(GameData{ id: id.clone(), systems: HashMap::new() })),
        clients: RwLock::new(HashMap::new()),
    };
    (id.clone(), game.start())
}

#[get("/{id}/players/")]
pub async fn get_players(state: web::Data<AppState>, info: web::Path<(GameID,)>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(Player::find_by_game(info.0, &state.db_pool).await))
}