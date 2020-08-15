use actix_web::{get, delete, web, HttpResponse};
use actix::prelude::*;
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use std::sync::{Arc, RwLock};
use std::collections::{HashMap};
use std::time::Duration;
use chrono::{DateTime, Utc};
use futures::executor::block_on;
use crate::{
    lib::{
        Result,
        error::{InternalError, ServerError},
        auth::Claims,
    },
    game::{
        faction::{FactionID, GameFaction, generate_game_factions},
        fleet::{
            fleet::{Fleet, FleetID, FLEET_RANGE},
            ship::{ShipQueue, ShipQueueID, ShipGroup, ShipGroupID},
        },
        lobby::Lobby,
        player::{PlayerID, Player, init_player_wallets},
        system::{
            building::{Building, BuildingID, BuildingStatus, BuildingKind},
            system::{System, SystemID, assign_systems, generate_systems, init_player_systems}
        },
    },
    ws::{ client::ClientSession, protocol},
    AppState,
};
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Error, Transaction};
use sqlx_core::row::Row;

pub const GAME_START_WALLET: usize = 200;
pub const VICTORY_POINTS: u16 = 300;
pub const VICTORY_POINTS_PER_MINUTE: u16 = 10;

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct GameID(pub Uuid);

#[derive(Serialize, Deserialize, Clone)]
pub struct Game {
    pub id: GameID,
    pub game_speed: GameOptionSpeed,
    pub map_size: GameOptionMapSize
}

pub struct GameServer {
    pub id: GameID,
    state: web::Data<AppState>,
    clients: RwLock<HashMap<PlayerID, actix::Addr<ClientSession>>>,
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, PartialEq, sqlx::Type)]
#[sqlx(rename = "VARCHAR")]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum GameOptionSpeed {
    Slow,
    Medium,
    Fast,
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, PartialEq, sqlx::Type)]
#[sqlx(rename = "VARCHAR")]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum GameOptionMapSize {
    VerySmall,
    Small,
    Medium,
    Large,
    VeryLarge,
}

impl From<GameID> for Uuid {
    fn from(gid: GameID) -> Self { gid.0 }
}

impl<'a> FromRow<'a, PgRow<'a>> for Game {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        let id : Uuid = row.try_get("id")?;

        Ok(Game {
            id: GameID(id),
            game_speed: row.try_get("game_speed")?,
            map_size: row.try_get("map_size")?
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
    pub async fn find(gid: GameID, db_pool: &PgPool) -> Result<Self> {
        sqlx::query_as("SELECT * FROM game__games WHERE id = $1")
            .bind(Uuid::from(gid))
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(InternalError::GameUnknown))
    }

    pub async fn create(game: Game, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("INSERT INTO game__games(id, game_speed, map_size) VALUES($1, $2, $3)")
            .bind(Uuid::from(game.id))
            .bind(game.game_speed)
            .bind(game.map_size)
            .execute(tx).await.map_err(ServerError::from)
    }

    pub async fn remove(gid: GameID, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("DELETE FROM game__games WHERE id = $1")
            .bind(Uuid::from(gid))
            .execute(tx).await.map_err(ServerError::from)
    }
}

impl GameServer {
    async fn init(&mut self) -> Result<()> {
        generate_game_factions(self.id.clone(), &self.state.db_pool).await?;

        let game = Game::find(self.id.clone(), &self.state.db_pool).await?;

        let mut systems = generate_systems(self.id.clone(), game.map_size).await?;
        let mut players = Player::find_by_game(self.id, &self.state.db_pool).await?;
        assign_systems(&players, &mut systems).await?;
        init_player_wallets(&mut players, &self.state.db_pool).await?;
        System::insert_all(systems.clone(), &self.state.db_pool).await?;
        init_player_systems(&systems, game.game_speed, &self.state.db_pool).await?;
        
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

    async fn faction_broadcast(&self, fid: FactionID, message: protocol::Message) -> Result<()> {
        let ids: Vec<PlayerID> = Player::find_by_faction(fid, &self.state.db_pool).await?.iter().map(|p| p.id).collect();
        let clients = self.clients.read().expect("Poisoned lock on game clients");
        for (pid, c) in clients.iter() {
            if ids.contains(&pid) {
                c.do_send(message.clone());
            }
        }
        Ok(())
    }

    async fn produce_income(&mut self) -> Result<()> {
        let mut players: HashMap<PlayerID, Player> = Player::find_by_game(self.id.clone(), &self.state.db_pool).await?
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect();
        let mut players_income = HashMap::new();
        let mines: Vec<SystemID> = Building::find_by_kind(BuildingKind::Mine, &self.state.db_pool).await?
            .into_iter()
            .map(|b| b.system)
            .collect();

        // Add money to each player based on the number of
        // currently, the income is `some_player.income = some_player.number_of_systems_owned * 15`
        System::find_possessed(self.id.clone(), &self.state.db_pool).await?
            .into_iter()
            .for_each(|system| {
                let mut income = 10;
                if mines.contains(&system.id) {
                    income = 40;
                }
                *players_income.entry(system.player).or_insert(0) += income
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
                clients.get(&pid.unwrap()).map(|c| {
                    c.do_send(protocol::Message::new(
                        protocol::Action::PlayerIncome,
                        PlayerIncome{ income },
                        None,
                    ));
                });
            });
        }
        let mut tx =self.state.db_pool.begin().await?;
        for (_, p) in players {
            Player::update(p, &mut tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn process_fleet_arrival(&mut self, fleet_id: FleetID) -> Result<()> {
        let mut fleet = Fleet::find(&fleet_id, &self.state.db_pool).await?;
        fleet.ship_groups = ShipGroup::find_by_fleet(fleet.id.clone(), &self.state.db_pool).await?;
        let mut destination_system = System::find(fleet.destination_system.unwrap(), &self.state.db_pool).await?;
        let player = Player::find(fleet.player, &self.state.db_pool).await?;

        let system_owner = {
            match destination_system.player {
                Some(owner_id) => Some(Player::find(owner_id, &self.state.db_pool).await?),
                None => None,
            }
        };

        let result = destination_system.resolve_fleet_arrival(fleet, &player, system_owner, &self.state.db_pool).await?;
        
        self.ws_broadcast(result.into());
        
        Ok(())
    }

    async fn process_building_construction(&mut self, bid: BuildingID) -> Result<()> {
        let mut building = Building::find(bid, &self.state.db_pool).await?;
        let player = Player::find_system_owner(building.system.clone(), &self.state.db_pool).await?;

        building.status = BuildingStatus::Operational;

        let mut tx = self.state.db_pool.begin().await?;
        Building::update(building.clone(), &mut tx).await?;
        tx.commit().await?;

        self.faction_broadcast(player.faction.unwrap(), protocol::Message::new(
            protocol::Action::BuildingConstructed,
            building.clone(),
            None,
        )).await?;

        Ok(())
    }

    async fn process_ship_queue_production(&mut self, sqid: ShipQueueID) -> Result<()> {
        let ship_queue = ShipQueue::find(sqid, &self.state.db_pool).await?;
        let ship_group = ShipGroup::find_by_system_and_category(
            ship_queue.system.clone(),
            ship_queue.category.clone(),
            &self.state.db_pool
        ).await?;
        let player = Player::find_system_owner(ship_queue.system.clone(), &self.state.db_pool).await?;
        let mut tx = self.state.db_pool.begin().await?;

        if let Some(mut sg) = ship_group {
            sg.quantity += ship_queue.quantity;
            ShipGroup::update(&sg, &mut tx).await?;
        } else {
            let sg = ShipGroup{
                id: ShipGroupID(Uuid::new_v4()),
                system: Some(ship_queue.system.clone()),
                fleet: None,
                quantity: ship_queue.quantity.clone(),
                category: ship_queue.category.clone(),
            };
            ShipGroup::create(sg, &mut tx).await?;
        }
        
        ShipQueue::remove(ship_queue.id, &mut tx).await?;

        tx.commit().await?;

        let clients = self.clients.read().expect("Poisoned lock on game clients");
        clients.get(&player.id).unwrap().do_send(protocol::Message::new(
            protocol::Action::ShipQueueFinished,
            ship_queue.clone(),
            None,
        ));

        Ok(())
    }

    async fn distribute_victory_points(&mut self) -> Result<()> {
        let victory_systems = System::find_possessed_victory_systems(self.id.clone(), &self.state.db_pool).await?;
        let mut factions = GameFaction::find_all(self.id.clone(), &self.state.db_pool).await?
            .into_iter()    
            .map(|gf| (gf.faction.clone(), gf))
            .collect::<HashMap<FactionID, GameFaction>>();
        let mut players = Player::find_by_ids(victory_systems.clone().into_iter().map(|s| s.player.clone().unwrap()).collect(), &self.state.db_pool).await?
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect::<HashMap<PlayerID, Player>>();

        for system in victory_systems {
            factions.get_mut(
                &players.get_mut(&system.player.unwrap())
                    .unwrap()
                    .faction
                    .unwrap()
            ).unwrap().victory_points += VICTORY_POINTS_PER_MINUTE; 
        }

        let mut victorious_faction: Option<GameFaction> = None;
        let mut tx = self.state.db_pool.begin().await?;
        for (_, f) in factions.clone() {
            GameFaction::update(&f, &mut tx).await?;
            if f.victory_points >= VICTORY_POINTS {
                victorious_faction = Some(f.clone());
            }
        }
        tx.commit().await?;

        self.ws_broadcast(protocol::Message::new(
            protocol::Action::FactionPointsUpdated,
            factions.clone(),
            None
        ));

        if let Some(f) = victorious_faction {
            self.process_victory(&f, factions.values().cloned().collect::<Vec<GameFaction>>()).await?;
        }

        Ok(())
    }

    async fn process_victory(&mut self, victorious_faction: &GameFaction, factions: Vec<GameFaction>) -> Result<()> {
        #[derive(Serialize, Clone)]
        struct VictoryData {
            victorious_faction: FactionID,
            scores: Vec<GameFaction>
        }
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::Victory,
            VictoryData{
                victorious_faction: victorious_faction.faction,
                scores: factions,
            },
            None,
        ));
        self.state.clear_game(self.id).await?;
        Ok(())
    }

    pub async fn remove_player(&self, pid: PlayerID) -> Result<actix::Addr<ClientSession>> {
        let mut player = Player::find(pid, &self.state.db_pool).await?;
        player.is_connected = false;
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::PlayerLeft,
            pid.clone(),
            Some(pid),
        ));
        let mut clients = self.clients.write().expect("Poisoned lock on game players");
        let client = clients.get(&pid).unwrap().clone();
        clients.remove(&pid);
        Ok(client)
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
        ctx.run_later(Duration::new(4, 0), |this, _| this.begin());
        ctx.run_interval(Duration::new(5, 0), move |this, _| {
            let result = block_on(this.produce_income()).map_err(ServerError::from);
            if result.is_err() {
                println!("{:?}", result.err());
            }
        });
        ctx.run_interval(Duration::new(60, 0), move |this, _| {
            let result = block_on(this.distribute_victory_points()).map_err(ServerError::from);
            if result.is_err() {
                println!("{:?}", result.err());
            }
        });
    }
}

#[derive(actix::Message, Serialize, Clone)]
#[rtype(result="Arc<(actix::Addr<ClientSession>, bool)>")]
pub struct GameRemovePlayerMessage(pub PlayerID);

#[derive(actix::Message)]
#[rtype(result="()")]
pub struct GameFleetTravelMessage{
    pub fleet: Fleet
}

#[derive(actix::Message)]
#[rtype(result="()")]
pub struct GameShipQueueMessage{
    pub ship_queue: ShipQueue
}

#[derive(actix::Message)]
#[rtype(result="()")]
pub struct GameBuildingConstructionMessage{
    pub building: Building
}

#[derive(actix::Message)]
#[rtype(result="()")]
pub struct GameEndMessage{}

impl Handler<GameRemovePlayerMessage> for GameServer {
    type Result = Arc<(actix::Addr<ClientSession>, bool)>;

    fn handle(&mut self, GameRemovePlayerMessage(pid): GameRemovePlayerMessage, _ctx: &mut Self::Context) -> Self::Result {
        let client = block_on(self.remove_player(pid)).unwrap();
        Arc::new((client, self.is_empty()))
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
        let datetime: DateTime<Utc> = msg.fleet.destination_arrival_date.unwrap().into();
        ctx.run_later(datetime.signed_duration_since(Utc::now()).to_std().unwrap(), move |this, _| {
            let res = block_on(this.process_fleet_arrival(msg.fleet.id.clone()));
            if res.is_err() {
                println!("Fleet arrival fail : {:?}", res.err());
            }
        });
    }
}

impl Handler<GameShipQueueMessage> for GameServer {
    type Result = ();

    fn handle(&mut self, msg: GameShipQueueMessage, ctx: &mut Self::Context) -> Self::Result {
        let datetime: DateTime<Utc> = msg.ship_queue.finished_at.into();
        ctx.run_later(datetime.signed_duration_since(Utc::now()).to_std().unwrap(), move |this, _| {
            let res = block_on(this.process_ship_queue_production(msg.ship_queue.id.clone()));
            if res.is_err() {
                println!("Ship queue production failed : {:?}", res.err());
            }
        });
    }
}

impl Handler<GameBuildingConstructionMessage> for GameServer {
    type Result = ();

    fn handle(&mut self, msg: GameBuildingConstructionMessage, ctx: &mut Self::Context) -> Self::Result {
        let datetime: DateTime<Utc> = msg.building.built_at.into();
        ctx.run_later(datetime.signed_duration_since(Utc::now()).to_std().unwrap(), move |this, _| {
            let res = block_on(this.process_building_construction(msg.building.id.clone()));
            if res.is_err() {
                println!("Building construction failed : {:?}", res.err());
            }
        });
    }
}

impl Handler<GameEndMessage> for GameServer {
    type Result = ();

    fn handle(&mut self, _msg: GameEndMessage, ctx: &mut Self::Context) -> Self::Result {
        let clients = self.clients.read().expect("Poisoned lock on game clients");
        for (pid, c) in clients.iter() {
            self.state.add_client(&pid, c.clone());
        }
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
    let game = Game{
        id: id.clone(),
        game_speed: lobby.game_speed.clone(),
        map_size: lobby.map_size.clone(),
    };

    let mut tx = state.db_pool.begin().await?;
    Game::create(game, &mut tx).await?;
    tx.commit().await?;

    Player::transfer_from_lobby_to_game(&lobby.id, &id, &state.db_pool).await?;

    Ok((id, game_server.start()))
}

#[get("/{id}/players/")]
pub async fn get_players(state: web::Data<AppState>, info: web::Path<(GameID,)>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(Player::find_by_game(info.0, &state.db_pool).await?))
}

#[delete("/{id}/players/")]
pub async fn leave_game(state:web::Data<AppState>, claims: Claims, info: web::Path<(GameID,)>)
    -> Result<HttpResponse>
{
    let game = Game::find(info.0, &state.db_pool).await?;
    let mut player = Player::find(claims.pid, &state.db_pool).await?;

    if player.game != Some(game.id) {
        Err(InternalError::NotInLobby)?
    }
    player.reset(&state.db_pool).await?;

    let games = state.games();
    let game_server = games.get(&game.id).expect("Game exists in DB but not in HashMap");
    let (client, is_empty) = Arc::try_unwrap(game_server.send(GameRemovePlayerMessage(player.id.clone())).await?).ok().unwrap();
    state.add_client(&player.id, client.clone());
    if is_empty {
        drop(games);
        state.clear_game(game.id.clone()).await?;
    }
    Ok(HttpResponse::NoContent().finish())
}

#[get("/constants/")]
pub async fn get_game_constants() -> Result<HttpResponse> {
    #[derive(Serialize, Clone)]
    pub struct GameConstants {
        fleet_range: f64,
        victory_points_per_minute: u16,
        victory_points: u16,
    }
    Ok(HttpResponse::Ok().json(GameConstants{
        fleet_range: FLEET_RANGE,
        victory_points_per_minute: VICTORY_POINTS_PER_MINUTE,
        victory_points: VICTORY_POINTS,
    }))
}