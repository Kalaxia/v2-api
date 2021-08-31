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
        log::log,
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
        game::{
            game::{Game, GameID},
            victory::{check_forfeit_victory, distribute_victory_points},
        },
        player::player::{PlayerID, Player, init_player_wallets},
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

    fn get_task_duration(&self) -> Option<Duration> {
        let datetime: DateTime<Utc> = self.get_task_end_time().into();
        datetime.signed_duration_since(Utc::now()).to_std().ok()
    }
}

impl Handler<protocol::Message> for GameServer {
    type Result = ();

    fn handle(&mut self, msg: protocol::Message, _ctx: &mut Self::Context) -> Self::Result {
        block_on(self.ws_broadcast(&msg));
    }
}

impl Actor for GameServer {
    type Context = Context<Self>;
    
    fn started(&mut self, ctx: &mut Context<Self>) {
        block_on(self.ws_broadcast(&protocol::Message::new(
            protocol::Action::LobbyLaunched,
            self.id.clone(),
            None,
        )));
        
        self.add_task(ctx, "init".to_string(), Duration::new(1, 0), |this, _| block_on(this.init()));
        self.add_task(ctx, "begin".to_string(), Duration::new(4, 0), |this, _| block_on(this.begin()));
        run_interval(ctx, Duration::new(5, 0), move |this, _| {
            block_on(this.produce_income())
        });
        run_interval(ctx, Duration::new(60, 0), move |mut this, _| {
            block_on(distribute_victory_points(&mut this))
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
        System::insert_all(systems.iter(), &self.state.db_pool).await?;
        init_player_systems(&systems, game.game_speed, &self.state.db_pool).await?;
        
        self.ws_broadcast(&protocol::Message::new(
            protocol::Action::SystemsCreated,
            (),
            None
        )).await
    }

    async fn begin(&self) -> Result<()> {
        let game = Game::find(self.id.clone(), &self.state.db_pool).await?;
        #[derive(Serialize)]
        struct GameData{
            victory_points: i32
        }
        self.ws_broadcast(&protocol::Message::new(
            protocol::Action::GameStarted,
            GameData{
                victory_points: game.victory_points
            },
            None
        )).await
    }

    fn clients(&self) -> std::sync::RwLockReadGuard<HashMap<PlayerID, actix::Addr<ClientSession>>> {
        self.clients.read().expect("Poisoned lock on game clients")
    }

    pub async fn ws_broadcast(&self, message: &protocol::Message) -> Result<()> {
        let clients = self.clients();
        for pid in Player::find_ids_by_game(self.id, &self.state.db_pool).await? {
            self.ws_send(&clients, &pid, message);
        }
        Ok(())
    }

    pub async fn faction_broadcast(&self, fid: FactionID, message: protocol::Message) -> Result<()> {
        let clients = self.clients();
        for pid in Player::find_ids_by_game_and_faction(self.id, fid, &self.state.db_pool).await? {
            self.ws_send(&clients, &pid, &message);
        }
        Ok(())
    }

    pub fn player_broadcast(&self, pid: &PlayerID, message: &protocol::Message) {
        let clients = self.clients();
        self.ws_send(&clients, pid, message);
    }

    pub fn ws_send(&self, clients: &std::sync::RwLockReadGuard<HashMap<PlayerID, actix::Addr<ClientSession>>>, pid: &PlayerID, message: &protocol::Message) {
        let mut missing_messages = self.state.missing_messages_mut();

        if let Some(client) = clients.get(pid) {
            client.do_send(message.clone());
        } else {
            missing_messages.entry(*pid)
                .or_default()
                .push(message.clone());
        }
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
                *players_income.entry(system.player).or_insert(0) += income;
            }); // update the player's income

        // Notify the player for wallet update
        #[derive(Serialize, Clone)]
        struct PlayerIncome {
            income: usize
        }
        let clients = self.clients.read().expect("Poisoned lock on game clients");
        for (pid, income) in players_income {
            if let Some(p) = players.get_mut(&pid.unwrap()) {
                p.wallet += income;
                if let Some(c) = clients.get(&pid.unwrap()){
                    c.do_send(protocol::Message::new(
                        protocol::Action::PlayerIncome,
                        PlayerIncome{ income },
                        None,
                    ));
                }
            }
        }
        let mut tx = self.state.db_pool.begin().await?;
        for p in players.values() {
            p.update(&mut tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn remove_player(&self, pid: PlayerID) -> Result<Option<actix::Addr<ClientSession>>> {
        let mut player = Player::find(pid, &self.state.db_pool).await?;
        player.is_connected = false;
        player.update(&mut &self.state.db_pool).await?;
        self.ws_broadcast(&protocol::Message::new(
            protocol::Action::PlayerLeft,
            pid.clone(),
            Some(pid),
        )).await?;

        let mut clients = self.clients.write().expect("Poisoned lock on game players");
        Ok(clients.remove(&pid))
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
                    log(
                        gelf::Level::Error,
                        &format!("Task {} failed", task_name),
                        &format!("{:?}", result.err()),
                        vec![],
                        &this.state.logger
                    );
                }
            }
        ));
    }

    pub fn cancel_task(&mut self, task_name: &str, context: &mut actix::Context<GameServer>) {
        if let Some(task) = self.tasks.get(task_name) {
            context.cancel_future(*task);

            self.remove_task(task_name);
        }
    }

    pub fn remove_task(&mut self, task_name: &str) {
        self.tasks.remove(task_name);
    }

    pub fn is_empty(&self) -> bool {
        let clients = self.clients.read().expect("Poisoned lock on game players");
        
        clients.len() == 0
    }
}

#[derive(actix::Message, Serialize, Clone)]
#[rtype(result="Arc<(Option<actix::Addr<ClientSession>>, bool)>")]
pub struct GameRemovePlayerMessage(pub PlayerID);

#[derive(actix::Message, Clone)]
#[rtype(result="()")]
pub struct GameAddClientMessage(pub PlayerID, pub actix::Addr<ClientSession>);

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
    task_duration: Option<Duration>,
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
    pub fn new<F:FnOnce(&GameServer) -> Result<()> + Send + 'static>(task_id: String, task_duration:Option<Duration>, callback: F) -> Self {
        Self {
            task_id,
            task_duration,
            callback : Box::new(callback),
        }
    }
}

impl GameCancelTaskMessage
{
    pub const fn new(task_id: String) -> Self {
        Self {
            task_id,
        }
    }
}

#[derive(actix::Message)]
#[rtype(result="()")]
pub struct GameEndMessage{}

impl Handler<GameAddClientMessage> for GameServer {
    type Result = ();

    fn handle(&mut self, GameAddClientMessage(pid, client): GameAddClientMessage, _ctx: &mut Self::Context) -> Self::Result {
        let mut clients = self.clients.write().expect("Poisoned lock on game players");
        clients.insert(pid, client);
    }
}

impl Handler<GameRemovePlayerMessage> for GameServer {
    type Result = Arc<(Option<actix::Addr<ClientSession>>, bool)>;

    fn handle(&mut self, GameRemovePlayerMessage(pid): GameRemovePlayerMessage, ctx: &mut Self::Context) -> Self::Result {
        let client = block_on(self.remove_player(pid)).unwrap();
        ctx.run_later(Duration::new(5, 0), move |this, _| {
            let res = block_on(check_forfeit_victory(&this));
            if res.is_err() {
                println!("Error during forfeit victory check : {:?}", res.err());
            }
        });
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
        block_on(self.ws_broadcast(&protocol::Message::new(
            protocol::Action::FleetSailed,
            msg.fleet.clone(),
            Some(msg.fleet.player),
        )));
        // In this case, there is no battle, but a in-progress conquest
        // We update the conquest or cancel it depending on the remaining fleets
        if let Some(mut conquest) = block_on(Conquest::find_current_by_system(&msg.system.id, &self.state.db_pool)).map_err(ServerError::from).ok().unwrap() {
            block_on(conquest.remove_fleet(&msg.system, &msg.fleet, &self)).map_err(ServerError::from).ok().unwrap();
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
            msg.task_duration.unwrap_or(Duration::new(0, 0)),
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

fn run_interval<F>(
    ctx: &mut <GameServer as Actor>::Context,
    duration: Duration,
    mut closure: F
)
    where F: FnMut(&mut GameServer, & <GameServer as Actor>::Context) -> Result<()> + 'static,
{
    ctx.run_interval(duration, move |this, ctx| {
        let result = closure(this, ctx).map_err(ServerError::from);
        if result.is_err() {
            println!("{:?}", result.err());
        }
    });
}