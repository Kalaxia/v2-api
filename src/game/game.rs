use actix_web::{get, web, HttpResponse};
use actix::prelude::*;
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use std::sync::RwLock;
use std::collections::{HashMap};
use std::time::Duration;
use futures::executor::block_on;
use crate::{
    lib::{Result, error::ServerError},
    game::{
        faction::{FactionID},
        fleet::{
            fleet::{Fleet, FleetID, FLEET_TRAVEL_TIME},
        },
        lobby::Lobby,
        player::{PlayerID, Player},
        system::{System, SystemDominion, FleetArrivalOutcome, assign_systems, generate_systems}
    },
    ws::{ client::ClientSession, protocol},
    AppState,
};
use sqlx::{PgPool, postgres::{PgRow}, FromRow, Error};
use sqlx_core::row::Row;

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct GameID(pub Uuid);

#[derive(Serialize, Deserialize, Clone)]
pub struct Game {
    pub id: GameID,
}

pub struct GameServer {
    pub id: GameID,
    state: web::Data<AppState>,
    clients: RwLock<HashMap<PlayerID, actix::Addr<ClientSession>>>,
}

pub const MAP_SIZE: u16 = 10;
pub const TERRITORIAL_DOMINION_RATE: u8 = 60;

impl From<GameID> for Uuid {
    fn from(gid: GameID) -> Self { gid.0 }
}

impl<'a> FromRow<'a, PgRow<'a>> for Game {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        let id : Uuid = row.try_get("id")?;

        Ok(Game {
            id: GameID(id),
        })
    }
}

impl Handler<protocol::Message> for GameServer {
    type Result = ();

    fn handle(&mut self, msg: protocol::Message, _ctx: &mut Self::Context) -> Self::Result {
        self.ws_broadcast(msg);
    }
}

impl Game {
    pub async fn create(game: Game, db_pool: &PgPool) -> Result<u64> {
        sqlx::query("INSERT INTO game__games(id) VALUES($1)")
            .bind(Uuid::from(game.id))
            .execute(db_pool).await.map_err(ServerError::from)
    }

    pub async fn remove(gid: GameID, db_pool: &PgPool) -> Result<u64> {
        sqlx::query("DELETE FROM game__games WHERE id = $1")
            .bind(Uuid::from(gid))
            .execute(db_pool).await.map_err(ServerError::from)
    }
}

impl GameServer {
    async fn init(&mut self) -> Result<()> {
        let mut g = generate_systems(self.id.clone()).await?;
        let players = Player::find_by_game(self.id, &self.state.db_pool).await;
        assign_systems(players, &mut g).await?;

        let (nodes, _) = g.into_nodes_edges();
        let systems = nodes.into_iter().map(|n| n.weight);
        System::insert_all(systems, &self.state.db_pool).await?;
        
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::SystemsCreated,
            (),
            None
        ));

        Ok(())
    }

    fn begin(&self) {
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::GameStarted,
            (),
            None
        ));
    }

    fn ws_broadcast(&self, message: protocol::Message) {
        let clients = self.clients.read().expect("Poisoned lock on game clients");
        for (_, c) in clients.iter() {
            c.do_send(message.clone());
        }
    }

    async fn produce_income(&mut self) -> Result<()> {
        let mut players: HashMap<PlayerID, Player> = Player::find_by_game(self.id.clone(), &self.state.db_pool).await
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect();
        let mut players_income = HashMap::new();

        // Add money to each player based on the number of
        // currently, the income is `some_player.income = some_player.number_of_systems_owned * 15`
        System::find_possessed(self.id.clone(), &self.state.db_pool).await
            .into_iter()
            .for_each(|system| {
                *players_income.entry(system.player).or_insert(0) += 15
            }); // update the player's income

        // Notify the player for wallet update
        #[derive(Serialize, Clone)]
        struct PlayerIncome {
            income: usize
        }
        let clients = self.clients.read().expect("Poisoned lock on game clients");
        for (pid, income) in players_income {
            players.get_mut(&pid.unwrap()).map(|p| {
                p.wallet += income;
                clients.get(&pid.unwrap()).unwrap().do_send(protocol::Message::new(
                    protocol::Action::PlayerIncome,
                    PlayerIncome{ income },
                    None,
                ));
            });
        }
        for (_, p) in players {
            Player::update(p, &self.state.db_pool).await?;
        }
        Ok(())
    }

    async fn process_fleet_arrival(&mut self, fleet_id: FleetID) -> Result<()> {
        let fleet = Fleet::find(&fleet_id, &self.state.db_pool).await?;
        let mut destination_system = System::find(fleet.destination_system.unwrap(), &self.state.db_pool).await?;
        let player = Player::find(fleet.player, &self.state.db_pool).await?;

        let system_owner = {
            match destination_system.player {
                Some(owner_id) => Some(Player::find(owner_id, &self.state.db_pool).await?),
                None => None,
            }
        };

        let result = destination_system.resolve_fleet_arrival(fleet, &player, system_owner, &self.state.db_pool).await?;
        self.ws_broadcast(result.clone().into());
        if let FleetArrivalOutcome::Conquerred{ fleet: _fleet, system: _system } = result {
            self.check_victory().await?;
        }
        Ok(())
    }

    async fn check_victory(&mut self) -> Result<()> {
        let total_territories = System::count(self.id.clone(), &self.state.db_pool).await as f64;
        let nb_territories_to_win = (total_territories * (TERRITORIAL_DOMINION_RATE as f64) / 100.0).ceil() as u32;
        let faction_systems_count = System::count_by_faction(self.id.clone(), &self.state.db_pool).await;

        for system_dominion in faction_systems_count.iter() {
            if system_dominion.nb_systems >= nb_territories_to_win {
                self.process_victory(system_dominion, faction_systems_count.clone()).await?;
                break;
            }
        }
        Ok(())
    }

    async fn process_victory(&mut self, system_dominion: &SystemDominion, faction_systems_count: Vec<SystemDominion>) -> Result<()> {
        #[derive(Serialize, Clone)]
        struct VictoryData {
            victorious_faction: FactionID,
            scores: Vec<SystemDominion>
        }
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::Victory,
            VictoryData{
                victorious_faction: system_dominion.faction_id,
                scores: faction_systems_count,
            },
            None,
        ));
        self.state.clear_game(self.id).await?;
        Ok(())
    }

    pub async fn remove_player(&self, pid: PlayerID) -> Result<()> {
        let mut player = Player::find(pid, &self.state.db_pool).await?;
        player.is_connected = false;
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::PlayerLeft,
            pid.clone(),
            Some(pid),
        ));
        let mut clients = self.clients.write().expect("Poisoned lock on game players");
        clients.remove(&pid);
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        let clients = self.clients.read().expect("Poisoned lock on game players");
        
        clients.len() == 0
    }
}

impl Actor for GameServer {
    type Context = Context<Self>;
    
    fn started(&mut self, ctx: &mut Context<Self>) {
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::LobbyLaunched,
            self.id.clone(),
            None,
        ));
        ctx.run_later(Duration::new(1, 0), |this, _| {
            let result = block_on(this.init()).map_err(ServerError::from);
            if result.is_err() {
                println!("{:?}", result.err());
            }
        });
        ctx.run_later(Duration::new(5, 0), |this, _| this.begin());
        ctx.run_interval(Duration::new(6, 0), move |this, _| {
            let result = block_on(this.produce_income()).map_err(ServerError::from);
            if result.is_err() {
                println!("{:?}", result.err());
            }
        });
    }
 
    fn stopped(&mut self, _ctx: &mut Context<Self>) {
        let clients = self.clients.read().expect("Poisoned lock on game clients");
        for (pid, c) in clients.iter() {
            self.state.add_client(&pid, c.clone());
        }
    }
}

#[derive(actix::Message, Serialize, Clone)]
#[rtype(result="bool")]
pub struct GameRemovePlayerMessage(pub PlayerID);

#[derive(actix::Message)]
#[rtype(result="()")]
pub struct GameFleetTravelMessage{
    pub fleet: Fleet
}

#[derive(actix::Message)]
#[rtype(result="()")]
pub struct GameEndMessage{}

impl Handler<GameRemovePlayerMessage> for GameServer {
    type Result = bool;

    fn handle(&mut self, GameRemovePlayerMessage(pid): GameRemovePlayerMessage, ctx: &mut Self::Context) -> Self::Result {
        block_on(self.remove_player(pid));
        self.is_empty()
    }
}

impl Handler<GameFleetTravelMessage> for GameServer {
    type Result = ();

    fn handle(&mut self, msg: GameFleetTravelMessage, ctx: &mut Self::Context) -> Self::Result {
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::FleetSailed,
            msg.fleet.clone(),
            Some(msg.fleet.player),
        ));
        ctx.run_later(Duration::new(FLEET_TRAVEL_TIME.into(), 0), move |this, _| {
            block_on(this.process_fleet_arrival(msg.fleet.id.clone()));
        });
    }
}

impl Handler<GameEndMessage> for GameServer {
    type Result = ();

    fn handle(&mut self, _msg: GameEndMessage, ctx: &mut Self::Context) -> Self::Result {
        ctx.stop();
        ctx.terminate();
    }
}

pub async fn create_game(lobby: &Lobby, state: web::Data<AppState>, clients: HashMap<PlayerID, actix::Addr<ClientSession>>) -> Result<(GameID, Addr<GameServer>)> {
    let id = GameID(Uuid::new_v4());
    
    let game_server = GameServer{
        id: id.clone(),
        state: state.clone(),
        clients: RwLock::new(clients),
    };
    let game = Game{ id: id.clone() };

    Game::create(game, &state.db_pool).await?;

    Player::transfer_from_lobby_to_game(&lobby.id, &id, &state.db_pool).await?;

    Ok((id, game_server.start()))
}

#[get("/{id}/players/")]
pub async fn get_players(state: web::Data<AppState>, info: web::Path<(GameID,)>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(Player::find_by_game(info.0, &state.db_pool).await))
}
