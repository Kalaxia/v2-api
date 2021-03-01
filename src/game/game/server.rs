use actix_web::web;
use actix::prelude::*;
use serde::{Serialize};
use std::sync::Arc;
use std::collections::HashMap;
use std::time::Duration;
use chrono::{DateTime, Utc};
use futures::executor::block_on;
use crate::{
    lib::{
        Result,
        error::ServerError,
    },
    game::{
        faction::{FactionID, GameFaction, generate_game_factions},
        fleet::{
            fleet::{Fleet, FleetID},
            squadron::{FleetSquadron},
        },
        game::game::{Game, GameID, VICTORY_POINTS_PER_MINUTE},
        ship::queue::ShipQueue,
        player::{PlayerID, Player, init_player_wallets},
        system::{
            building::{Building, BuildingID, BuildingStatus, BuildingKind},
            system::{System, SystemID, assign_systems, generate_systems, init_player_systems}
        },
    },
    ws::{ client::ClientSession, protocol},
    AppState,
};

pub struct GameServer {
    pub id: GameID,
    pub state: web::Data<AppState>,
    pub clients: HashMap<PlayerID, actix::Addr<ClientSession>>,
}

impl Handler<protocol::Message> for GameServer {
    type Result = ();

    fn handle(&mut self, msg: protocol::Message, _ctx: &mut Self::Context) -> Self::Result {
        self.ws_broadcast(msg);
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
        ctx.run_later(Duration::new(4, 0), |this, _| {
            let result = block_on(this.begin());
            if result.is_err() {
                println!("{:?}", result.err());
            }
        });
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

impl GameServer {
    async fn init(&mut self) -> Result<()> {
        generate_game_factions(self.id.clone(), &self.state.db_pool).await?;

        let mut game = Game::find(self.id.clone(), &self.state.db_pool).await?;

        let (mut systems, nb_victory_systems) = generate_systems(self.id.clone(), game.map_size).await?;

        game.victory_points = nb_victory_systems as i32 * 100;

        Game::update(game.clone(), &self.state.db_pool).await?;

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

    async fn begin(&self) -> Result<()> {
        let game = Game::find(self.id.clone(), &self.state.db_pool).await?;
        #[derive(Serialize)]
        struct GameData{
            victory_points: i32
        }
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::GameStarted,
            GameData{
                victory_points: game.victory_points
            },
            None
        ));
        Ok(())
    }

    pub fn ws_broadcast(&self, message: protocol::Message) {
        for (_, c) in self.clients.iter() {
            c.do_send(message.clone());
        }
    }

    pub async fn ws_send(&self, pid: &PlayerID, message: protocol::Message) {
        self.clients.get(pid).unwrap().do_send(message);
    }

    async fn faction_broadcast(&self, fid: FactionID, message: protocol::Message) -> Result<()> {
        let ids: Vec<PlayerID> = Player::find_by_faction(fid, &self.state.db_pool).await?.iter().map(|p| p.id).collect();
        for (pid, c) in self.clients.iter() {
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
            .filter(|b| b.status == BuildingStatus::Operational)
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
        for (pid, income) in players_income {
            players.get_mut(&pid.unwrap()).map(|p| {
                p.wallet += income;
                self.clients.get(&pid.unwrap()).map(|c| {
                    c.do_send(protocol::Message::new(
                        protocol::Action::PlayerIncome,
                        PlayerIncome{ income },
                        None,
                    ));
                });
            });
        }
        let mut tx = self.state.db_pool.begin().await?;
        for (_, p) in players {
            p.update(&mut tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn process_fleet_arrival(&mut self, fleet_id: FleetID) -> Result<()> {
        let mut fleet = Fleet::find(&fleet_id, &self.state.db_pool).await?;
        fleet.squadrons = FleetSquadron::find_by_fleet(fleet.id.clone(), &self.state.db_pool).await?;
        let mut destination_system = System::find(fleet.destination_system.unwrap(), &self.state.db_pool).await?;
        let player = Player::find(fleet.player, &self.state.db_pool).await?;

        let system_owner = {
            match destination_system.player {
                Some(owner_id) => Some(Player::find(owner_id, &self.state.db_pool).await?),
                None => None,
            }
        };

        let result = destination_system.resolve_fleet_arrival(&self, fleet, &player, system_owner, &self.state.db_pool).await?;
        
        self.ws_broadcast(result.into());
        
        Ok(())
    }

    async fn process_building_construction(&mut self, bid: BuildingID) -> Result<()> {
        let mut building = Building::find(bid, &self.state.db_pool).await?;
        let player = Player::find_system_owner(building.system.clone(), &self.state.db_pool).await?;

        building.status = BuildingStatus::Operational;

        let mut tx = self.state.db_pool.begin().await?;
        building.update(&mut tx).await?;
        tx.commit().await?;

        self.faction_broadcast(player.faction.unwrap(), protocol::Message::new(
            protocol::Action::BuildingConstructed,
            building.clone(),
            None,
        )).await?;

        Ok(())
    }

    async fn distribute_victory_points(&mut self) -> Result<()> {
        let victory_systems = System::find_possessed_victory_systems(self.id.clone(), &self.state.db_pool).await?;
        let game = Game::find(self.id.clone(), &self.state.db_pool).await?;
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
            if f.victory_points >= game.victory_points {
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

        let game = Game::find(self.id, &self.state.db_pool).await?;
        self.state.clear_game(&game).await?;
        Ok(())
    }

    pub async fn remove_player(&mut self, pid: PlayerID) -> Result<actix::Addr<ClientSession>> {
        let mut player = Player::find(pid, &self.state.db_pool).await?;
        player.is_connected = false;
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::PlayerLeft,
            pid.clone(),
            Some(pid),
        ));
        let client = self.clients.get(&pid).unwrap().clone();
        self.clients.remove(&pid);
        Ok(client)
    }

    pub fn is_empty(&self) -> bool {
        self.clients.len() == 0
    }
}

#[derive(actix::Message, Serialize, Clone)]
#[rtype(result="Arc<(actix::Addr<ClientSession>, bool)>")]
pub struct GameRemovePlayerMessage(pub PlayerID);

#[derive(actix::Message, Serialize, Clone)]
#[rtype(result="()")]
pub struct GameNotifyPlayerMessage(pub PlayerID, pub protocol::Message);

#[derive(actix::Message, Serialize, Clone)]
#[rtype(result="()")]
pub struct GameNotifyFactionMessage(pub FactionID, pub protocol::Message);

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

impl Handler<GameNotifyPlayerMessage> for GameServer {
    type Result = ();

    fn handle(&mut self, msg: GameNotifyPlayerMessage, _ctx: &mut Self::Context) -> Self::Result {
        let client = self.clients.get(&msg.0).unwrap().clone();
        client.do_send(msg.1);
    }
}

impl Handler<GameNotifyFactionMessage> for GameServer {
    type Result = ();

    fn handle(&mut self, msg: GameNotifyFactionMessage, _ctx: &mut Self::Context) -> Self::Result {
        let res = block_on(self.faction_broadcast(msg.0, msg.1));
        if res.is_err() {
            println!("Faction broadcast failed : {:?}", res.err());
        }
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
            let res = block_on(ShipQueue::produce(&this, msg.ship_queue.id.clone()));
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
        for (pid, c) in self.clients.iter() {
            block_on(self.state.add_client(&pid, c.clone()));
        }
        ctx.stop();
        ctx.terminate();
    }
}
