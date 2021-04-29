use actix_web::web;
use actix::prelude::*;
use serde::{Serialize};
use std::sync::{Arc, RwLock};
use std::collections::{HashMap};
use std::time::Duration;
use chrono::{DateTime, Utc};
use futures::{
    executor::block_on,
};
use crate::{
    lib::{
        Result,
        error::ServerError,
        time::Time
    },
    game::{
        faction::{FactionID, GameFaction, generate_game_factions},
        fleet::{
            combat::conquest::Conquest,
            fleet::Fleet,
            travel::process_fleet_arrival,
        },
        game::game::{Game, GameID, VICTORY_POINTS_PER_MINUTE},
        player::{PlayerID, Player, init_player_wallets},
        system::{
            building::{Building, BuildingStatus, BuildingKind},
            system::{System, SystemID, assign_systems, generate_systems, init_player_systems}
        },
    },
    ws::{ client::ClientSession, protocol},
    AppState,
};

pub struct GameServer {
    pub id: GameID,
    pub state: web::Data<AppState>,
    pub clients: RwLock<HashMap<PlayerID, actix::Addr<ClientSession>>>,
    pub tasks: HashMap<String, actix::SpawnHandle>,
}

/// The trait of every type that can represent a task. A task is launched by message-passing to the
/// game server with [GameScheduleTaskMessage]. When handled, this message launches a timer, and
/// eventually perform the task by 
///
/// Each timer is named by `get_task_id` to allow players to cancel its associated task before it
/// triggers.
pub trait GameServerTask{
    fn get_task_id(&self) -> String;

    fn get_task_end_time(&self) -> Time;

    fn get_task_duration(&self) -> Duration {
        let datetime: DateTime<Utc> = self.get_task_end_time().into();
        datetime.signed_duration_since(Utc::now()).to_std().unwrap()
    }
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
        
        self.add_task(ctx, "init".to_string(), Duration::new(1, 0), |this, _| block_on(this.init()));
        self.add_task(ctx, "begin".to_string(), Duration::new(4, 0), |this, _| block_on(this.begin()));
        self.run_interval(ctx, Duration::new(5, 0), move |this, _| {
            block_on(this.produce_income())
        });
        self.run_interval(ctx, Duration::new(60, 0), move |this, _| {
            block_on(this.distribute_victory_points())
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
        let clients = self.clients.read().expect("Poisoned lock on game clients");
        for (_, c) in clients.iter() {
            c.do_send(message.clone());
        }
    }

    pub fn ws_send(&self, pid: &PlayerID, message: protocol::Message) {
        let clients = self.clients.read().expect("Poisoned lock on game clients");
        clients.get(pid).unwrap().do_send(message);
    }

    pub async fn faction_broadcast(&self, fid: FactionID, message: protocol::Message) -> Result<()> {
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
        let mut tx = self.state.db_pool.begin().await?;
        for (_, p) in players {
            p.update(&mut tx).await?;
        }
        tx.commit().await?;
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

        for system in victory_systems.iter() {
            factions.get_mut(
                &players.get_mut(&system.player.unwrap())
                    .unwrap()
                    .faction
                    .unwrap()
            ).unwrap().victory_points += VICTORY_POINTS_PER_MINUTE;
        }

        let mut victorious_faction: Option<&GameFaction> = None;
        let mut tx = self.state.db_pool.begin().await?;
        for f in factions.values() {
            GameFaction::update(f, &mut tx).await?;
            if f.victory_points >= game.victory_points {
                victorious_faction = Some(f);
            }
        }
        tx.commit().await?;

        self.ws_broadcast(protocol::Message::new(
            protocol::Action::FactionPointsUpdated,
            factions.clone(),
            None
        ));

        if let Some(f) = victorious_faction {
            self.process_victory(f, factions.values().cloned().collect::<Vec<GameFaction>>()).await?;
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

    pub fn run_interval<F>(
        &mut self,
        ctx: &mut <Self as Actor>::Context,
        duration: Duration,
        closure: F
    )
        where F: FnOnce(&mut Self, & <Self as Actor>::Context) -> Result<()> + 'static + Clone,
    {
        ctx.run_interval(duration, move |this, ctx| {
            let result = closure.clone()(this, ctx).map_err(ServerError::from);
            if result.is_err() {
                println!("{:?}", result.err());
            }
        });
    }

    pub fn add_task<F>(
        &mut self,
        ctx: &mut <Self as Actor>::Context,
        task_name: String,
        duration: Duration,
        closure: F
    )
        where F: 'static + FnOnce(&mut Self, & <Self as Actor>::Context) -> Result<()>,
    {
        self.tasks.insert(task_name.clone(), ctx.run_later(
            duration,
            move |this, ctx| {
                let result = closure(this, ctx).map_err(ServerError::from);
                this.remove_task(&task_name);
                if result.is_err() {
                    println!("{:?}", result.err());
                }
            }
        ));
    }

    pub fn cancel_task(&mut self, task_name: &String, context: &mut actix::Context<GameServer>) {
        if let Some(task) = self.tasks.get(task_name) {
            context.cancel_future(*task);

            self.remove_task(task_name);
        }
    }

    pub fn remove_task(&mut self, task_name: &String) {
        self.tasks.remove(task_name);
    }

    pub fn is_empty(&self) -> bool {
        let clients = self.clients.read().expect("Poisoned lock on game players");
        
        clients.len() == 0
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
    pub fleet: Fleet,
    pub system: System
}

/// Because of the genericity of [GameScheduleTaskMessage] we will have plenty of them to send.
/// This macro helps keeping the code readable:
/// ```ignore
/// let ship_queue = todo!();
/// server.do_send(task!(ship_queue -> move |gs: &GameServer| block_on(ship_queue.so_something)));
/// ```
#[macro_export]
macro_rules! task {
    ($data:ident -> $exp:expr) => {
        {
            use crate::game::game::server::{GameScheduleTaskMessage, GameServerTask};
            GameScheduleTaskMessage::new($data.get_task_id(), $data.get_task_duration(), $exp)
        }
    };
}

#[macro_export]
macro_rules! cancel_task {
    ($data:ident) => {
        {
            use crate::game::game::server::GameCancelTaskMessage;
            GameCancelTaskMessage::new($data.get_task_id())
        }
    };
}

/// A generic message type used to schedule very simple cancelable tasks (e.g. ship or building
/// production).
#[derive(actix::Message)]
#[rtype(result="()")]
pub struct GameScheduleTaskMessage
{
    task_id: String,
    task_duration: Duration,
    callback: Box<dyn FnOnce(&GameServer) -> Result<()> + Send + 'static>,
}

#[derive(actix::Message)]
#[rtype(result="()")]
pub struct GameCancelTaskMessage
{
    task_id: String,
}

impl GameScheduleTaskMessage
{
    pub fn new<F:FnOnce(&GameServer) -> Result<()> + Send + 'static>(task_id: String, task_duration:Duration, callback: F) -> Self {
        Self {
            task_id,
            task_duration,
            callback : Box::new(callback),
        }
    }
}

impl GameCancelTaskMessage
{
    pub fn new(task_id: String) -> Self {
        Self {
            task_id,
        }
    }
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
        let clients = self.clients.read().expect("Poisoned lock on game clients");
        let client = clients.get(&msg.0).unwrap().clone();
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
        // In this case, there is no battle, but a in-progress conquest
        // We update the conquest or cancel it depending on the remaining fleets
        if let Some(mut conquest) = block_on(Conquest::find_current_by_system(&msg.system.id, &self.state.db_pool)).map_err(ServerError::from).ok().unwrap() {
            let is_cancelled = block_on(conquest.remove_fleet(&msg.system, &msg.fleet, &self.state.db_pool)).map_err(ServerError::from).ok().unwrap();
            if is_cancelled {
                self.ws_broadcast(protocol::Message::new(
                    protocol::Action::ConquestCancelled,
                    conquest,
                    None
                ));
            } else {
                self.state.games().get(&self.id).unwrap().do_send(task!(conquest -> move |server| block_on(conquest.end(&server))));
            }
        }
        let datetime: DateTime<Utc> = msg.fleet.destination_arrival_date.unwrap().into();
        ctx.run_later(datetime.signed_duration_since(Utc::now()).to_std().unwrap(), move |this, _| {
            let res = block_on(process_fleet_arrival(&this, msg.fleet.id));
            if res.is_err() {
                println!("Fleet arrival fail : {:?}", res.err());
            }
        });
    }
}

impl Handler<GameScheduleTaskMessage> for GameServer
{
    type Result = ();

    fn handle(&mut self, msg: GameScheduleTaskMessage, mut ctx: &mut Self::Context) -> Self::Result {
        self.add_task(
            &mut ctx,
            msg.task_id.clone(),
            msg.task_duration,
            move |this, _| (msg.callback)(&this)
        )
    }
}

impl Handler<GameCancelTaskMessage> for GameServer
{
    type Result = ();

    fn handle(&mut self, msg: GameCancelTaskMessage, ctx: &mut Self::Context) -> Self::Result {
        self.cancel_task(&msg.task_id, ctx);
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
