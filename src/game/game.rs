use actix_web::{get, web, HttpResponse};
use actix::prelude::*;
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use std::collections::{HashMap};
use std::time::Duration;
use std::sync::{Arc, Mutex};
use crate::{
    lib::{Result, error::InternalError},
    game::{
        faction::{FactionID},
        fleet::{
            fleet::{Fleet, FleetID, FLEET_TRAVEL_TIME},
        },
        lobby::Lobby,
        player::{PlayerID, Player, PlayerData},
        system::{SystemID, System, generate_systems}
    },
    ws::protocol,
    AppState,
};

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct GameID(Uuid);

#[derive(Serialize, Deserialize, Clone)]
pub struct GameData {
    pub id: GameID,
    pub systems: HashMap<SystemID, System>
}

pub struct Game {
    players: Arc<Mutex<HashMap<PlayerID, Player>>>,
    data: Arc<Mutex<GameData>>
}

pub const MAP_SIZE: u8 = 10;

impl Game {
    fn init(&mut self) {
        let mut data = self.data.lock().expect("Poisoned lock on game data");
        (*data).systems = generate_systems();
        drop(data); 
        self.assign_systems();
    }

    fn begin(&self) {
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::GameStarted,
            self.data.lock().expect("Poisoned lock on game data").clone()
        ), None);
    }

    fn ws_broadcast(
        &self,
        message: protocol::Message,
        skip_id: Option<PlayerID>
    ) {
        let players = self.players.lock().expect("Poisoned lock on game players");
        for (id, player) in players.iter() {
            if Some(*id) != skip_id {
                player.websocket.as_ref().map(|ws| {
                    ws.do_send(message.clone());
                });
            }
        }
    }

    fn assign_systems(&mut self) {
        let mut placed_per_faction = HashMap::new();
        let players = self.players.lock().expect("Poisoned lock on game players");
        let mut data = self.data.lock().expect("Poisoned lock on game data");

        for p in players.values() {
            let fid = p.data.faction.unwrap().clone();
            let i = placed_per_faction.entry(fid).or_insert(0);
            let place = self.find_place(fid, *i, &data.systems);
            *i += 1;

            if let Some(place) = place {
                // legitimate use of unwrap, because we KNOW `place` IS an existing system id
                // if it is Some()
                data.systems.get_mut(&place).unwrap().player = Some(p.data.id);
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

    fn produce_income(&mut self) {
        let mut players_income = HashMap::new();
        let mut players = self.players.lock().expect("Poisoned lock on game players");
        let data = self.data.lock().expect("Poisoned lock on game data");
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
                p.data.wallet += income;
                p.websocket.as_ref().map(|ws| {
                    ws.do_send(protocol::Message::new(
                        protocol::Action::PlayerIncome,
                        PlayerIncome{ income }
                    ));
                });
            });
        }
    }

    fn process_fleet_arrival(&self, fleet_id: FleetID, system_id: SystemID) -> Result<()> {
        let mut data = self.data.lock().expect("Poisoned lock on game data");
        let fleet = {
            let system = data.systems.get_mut(&system_id).ok_or(InternalError::SystemUnknown)?;
            let f = system.fleets.get_mut(&fleet_id).ok_or(InternalError::FleetUnknown)?.clone();
            system.fleets.remove(&fleet_id.clone());
            f
        };
        let destination_system = data.systems.get_mut(&fleet.destination_system.unwrap()).ok_or(InternalError::SystemUnknown)?;

        let players = self.players.lock().expect("Poisoned lock on game players");
        let player = players.get(&fleet.player).ok_or(InternalError::PlayerUnknown)?;

        let system_owner = {
            match destination_system.player {
                Some(owner_id) => Some(players.get(&owner_id).ok_or(InternalError::PlayerUnknown)?),
                None => None,
            }
        };
        self.ws_broadcast(destination_system.resolve_fleet_arrival(fleet, player, system_owner).into(), None);
        Ok(())
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
        ctx.run_interval(Duration::new(5, 0), |this, _| this.produce_income());
    }
 
    fn stopped(&mut self, _ctx: &mut Context<Self>) {
        println!("Game is stopped");
    }
}

#[derive(Serialize, Clone)]
pub struct GamePlayersMessage{}
#[derive(Serialize, Clone)]
pub struct GameDataMessage{}

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

impl actix::Message for GamePlayersMessage {
    type Result = Arc<Mutex<HashMap<PlayerID, Player>>>;
}

impl actix::Message for GameDataMessage {
    type Result = Arc<Mutex<GameData>>;
}

impl Handler<GamePlayersMessage> for Game {
    type Result = Arc<Mutex<HashMap<PlayerID, Player>>>;

    fn handle(&mut self, _msg: GamePlayersMessage, _ctx: &mut Self::Context) -> Self::Result {
        self.players.clone()
    }
}

impl Handler<GameDataMessage> for Game {
    type Result = Arc<Mutex<GameData>>;

    fn handle(&mut self, _msg: GameDataMessage, _ctx: &mut Self::Context) -> Self::Result {
        self.data.clone()
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
            this.process_fleet_arrival(msg.fleet.id.clone(), msg.fleet.system.clone());
        });
    }
}

pub fn create_game(lobby: &Lobby, players: &mut HashMap<PlayerID, Player>) -> (GameID, Addr<Game>) {
    let id = GameID(Uuid::new_v4());
    let mut game_players = HashMap::new();
    
    for pid in &lobby.players {
        players.get_mut(pid).map(|p| {
            p.data.lobby = None;
            p.data.game = Some(id.clone());
            game_players.insert(p.data.id, p.clone())
        });
    }
    let game = Game{
        players: Arc::new(Mutex::new(game_players)),
        data: Arc::new(Mutex::new(GameData{ id: id.clone(), systems: HashMap::new() }))
    };
    (id.clone(), game.start())
}

#[get("/{id}/players/")]
pub async fn get_players(state: web::Data<AppState>, info: web::Path<(GameID,)>) -> Result<HttpResponse> {
    let games = state.games();
    let game = games.get(&info.0).ok_or(InternalError::GameUnknown)?;
    match game.send(GamePlayersMessage{}).await {
        Ok(locked_data) => {
            let players = locked_data.lock().expect("Poisoned lock on game players");
            Ok(HttpResponse::Ok().json((*players).iter().map(|(_, p)| p.data.clone()).collect::<Vec<PlayerData>>()))
        },
        _ => Ok(HttpResponse::InternalServerError().finish())
    }
}