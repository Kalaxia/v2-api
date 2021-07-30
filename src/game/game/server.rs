use actix::{fut::wrap_future, prelude::*};
use serde::{Serialize};
use std::collections::{HashMap};
use std::time::Duration;
use chrono::{DateTime, Utc};
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
    game::global::state,
};

pub struct GameServer {
    pub id: GameID,
    pub clients: HashMap<PlayerID, actix::Addr<ClientSession>>,
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

    fn handle(&mut self, msg: protocol::Message, ctx: &mut Self::Context) -> Self::Result {
        ctx.wait(wrap_future(Self::ws_broadcast(self.id, msg)).map(|_,_,_| ()));
    }
}

impl Actor for GameServer {
    type Context = Context<Self>;
    
    fn started(&mut self, ctx: &mut Context<Self>) {
        ctx.wait(
            wrap_future(Self::ws_broadcast(self.id, protocol::Message::new(
                protocol::Action::LobbyLaunched,
                self.id.clone(),
                None,
            ))).map(|_,_,_| { () })
        );
        
        self.add_task(ctx, "init".to_string(), Duration::new(1, 0), |this, ctx| { Ok(ctx.wait(this.init())) });
        self.add_task(ctx, "begin".to_string(), Duration::new(4, 0), |this, ctx| { Ok(ctx.wait(this.begin())) });
        run_interval(ctx, Duration::new(5, 0), move |this, ctx| {
            Ok(ctx.wait(wrap_future(Self::produce_income(this.id)).map(|_,_,_| ())))
        });
        run_interval(ctx, Duration::new(60, 0), move |this, ctx| {
            Ok(ctx.wait(wrap_future(Self::distribute_victory_points(this.id)).map(|_,_,_| ())))
        });
    }
}

impl GameServer {
    fn init(&mut self) -> impl ActorFuture<Actor=Self, Output=()> {
        let gid = self.id;
        wrap_future(async move {
            let state = state();
            generate_game_factions(gid, &state.db_pool).await?;

            let mut game = Game::find(gid, &state.db_pool).await?;

            let (mut systems, nb_victory_systems) = generate_systems(gid, game.map_size).await?;

            game.victory_points = nb_victory_systems as i32 * 100;

            Game::update(game.clone(), &state.db_pool).await?;

            let mut players = Player::find_by_game(gid, &state.db_pool).await?;
            assign_systems(&players, &mut systems).await?;
            init_player_wallets(&mut players, &state.db_pool).await?;
            System::insert_all(systems.iter(), &state.db_pool).await?;
            init_player_systems(&systems, game.game_speed, &state.db_pool).await?;
            
            Self::ws_broadcast(gid, protocol::Message::new(
                protocol::Action::SystemsCreated,
                (),
                None
            )).await
        }).map(|_,_,_| ())
    }

    fn begin(&self) -> impl ActorFuture<Actor=Self, Output=()> {
        let gid = self.id;
        wrap_future(async move {
            let state = state();
            let game = Game::find(gid, &state.db_pool).await.expect("Game not found");
            #[derive(Serialize)]
            struct GameData{
                victory_points: i32
            }
            Self::ws_broadcast(gid, protocol::Message::new(
                protocol::Action::GameStarted,
                GameData{
                    victory_points: game.victory_points
                },
                None
            )).await;
        })
    }

//    fn clients(&self) -> std::sync::RwLockReadGuard<HashMap<PlayerID, actix::Addr<ClientSession>>> {
//        self.clients.read().expect("Poisoned lock on game clients")
//    }

    pub async fn ws_broadcast(gid: GameID, message: protocol::Message) -> Result<()> {
        let state = state();
        for pid in Player::find_ids_by_game(gid, &state.db_pool).await? {
            state.ws_send(&pid, &message);
        }
        Ok(())
    }

    pub async fn faction_broadcast(gid: GameID, fid: FactionID, message: protocol::Message) -> Result<()> {
        let state = state();
        for pid in Player::find_ids_by_game_and_faction(gid, fid, &state.db_pool).await? {
            state.ws_send(&pid, &message);
        }
        Ok(())
    }

    pub fn player_broadcast(pid: &PlayerID, message: protocol::Message) {
        state().ws_send(pid, &message);
    }

    pub fn ws_send(&self, clients: &std::sync::RwLockReadGuard<HashMap<PlayerID, actix::Addr<ClientSession>>>, pid: &PlayerID, message: &protocol::Message) {
        let state = state();
        let mut missing_messages = state.missing_messages_mut();

        if let Some(client) = clients.get(pid) {
            client.do_send(message.clone());
        } else {
            missing_messages.entry(*pid)
                .or_default()
                .push(message.clone());
        }
    }

    async fn produce_income(gid: GameID) -> Result<()> {
        let state = state();
        let mut players: HashMap<PlayerID, Player> = Player::find_by_game(gid, &state.db_pool).await?
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect();
        let mut players_income = HashMap::new();
        let mines: Vec<SystemID> = Building::find_by_kind(BuildingKind::Mine, &state.db_pool).await?
            .into_iter()
            .filter(|b| b.status == BuildingStatus::Operational)
            .map(|b| b.system)
            .collect();

        // Add money to each player based on the number of
        // currently, the income is `some_player.income = some_player.number_of_systems_owned * 15`
        System::find_possessed(gid, &state.db_pool).await?
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
        for (pid, income) in players_income {
            if let Some(p) = players.get_mut(&pid.unwrap()) {
                p.wallet += income;
                state.ws_send(&pid.unwrap(), &protocol::Message::new(
                        protocol::Action::PlayerIncome,
                        PlayerIncome{ income },
                        None,
                    ));
            }
        }
        let mut tx = state.db_pool.begin().await?;
        for p in players.values() {
            p.update(&mut tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn distribute_victory_points(gid: GameID) -> Result<()> {
        let state = state();
        let victory_systems = System::find_possessed_victory_systems(gid, &state.db_pool).await?;
        let game = Game::find(gid, &state.db_pool).await?;
        let mut factions = GameFaction::find_all(gid, &state.db_pool).await?
            .into_iter()
            .map(|gf| (gf.faction.clone(), gf))
            .collect::<HashMap<FactionID, GameFaction>>();
        let mut players = Player::find_by_ids(victory_systems.clone().into_iter().map(|s| s.player.clone().unwrap()).collect(), &state.db_pool).await?
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
        let mut tx = state.db_pool.begin().await?;
        for f in factions.values() {
            GameFaction::update(f, &mut tx).await?;
            if f.victory_points >= game.victory_points {
                victorious_faction = Some(f);
            }
        }
        tx.commit().await?;

        Self::ws_broadcast(gid, protocol::Message::new(
            protocol::Action::FactionPointsUpdated,
            factions.clone(),
            None
        )).await?;

        if let Some(f) = victorious_faction {
            Self::process_victory(gid, f, factions.values().cloned().collect::<Vec<GameFaction>>()).await?;
        }

        Ok(())
    }

    async fn process_victory(gid: GameID, victorious_faction: &GameFaction, factions: Vec<GameFaction>) -> Result<()> {
        let state = state();
        #[derive(Serialize, Clone)]
        struct VictoryData {
            victorious_faction: FactionID,
            scores: Vec<GameFaction>
        }
        Self::ws_broadcast(gid, protocol::Message::new(
            protocol::Action::Victory,
            VictoryData{
                victorious_faction: victorious_faction.faction,
                scores: factions,
            },
            None,
        )).await?;

        let game = Game::find(gid, &state.db_pool).await?;
        state.clear_game(&game).await?;
        Ok(())
    }

    pub fn remove_player(gid: GameID, pid: PlayerID) -> impl ActorFuture<Actor=GameServer, Output=Result<Option<actix::Addr<ClientSession>>>> {
        wrap_future(async move {
            let state = state();
            let mut player = Player::find(pid, &state.db_pool).await.unwrap();
            player.is_connected = false;
            Self::ws_broadcast(gid, protocol::Message::new(
                protocol::Action::PlayerLeft,
                pid.clone(),
                Some(pid),
            )).await;
            player.id
        })
        .map(|pid, this:&mut Self, _| {
            Ok(this.clients.remove(&pid))
        })
    }

    pub fn add_task<F>(
        &mut self,
        ctx: &mut <Self as Actor>::Context,
        task_name: String,
        duration: Duration,
        closure: F
    )
        where F: 'static + FnOnce(&mut Self, &mut <Self as Actor>::Context) -> Result<()>,
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
        self.clients.len() == 0
    }
}

#[derive(actix::Message, Serialize, Clone)]
#[rtype(result="std::result::Result<(Option<actix::Addr<ClientSession>>, bool), ()>")]
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
/// server.do_send(task!(ship_queue -> move |gs, ctx| {
///     do_something()
/// }));
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
    callback: Box<dyn FnOnce(&mut GameServer, &mut Context<GameServer>) -> Result<()> + Send + 'static>,
}

#[derive(actix::Message)]
#[rtype(result="()")]
pub struct GameCancelTaskMessage
{
    task_id: String,
}

impl GameScheduleTaskMessage
{
    pub fn new<F:FnOnce(&mut GameServer, &mut Context<GameServer>) -> Result<()> + Send + 'static>(task_id: String, task_duration:Option<Duration>, callback: F) -> Self {
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
        self.clients.insert(pid, client);
    }
}

impl Handler<GameRemovePlayerMessage> for GameServer {
    type Result = ResponseActFuture<Self, std::result::Result<(Option<actix::Addr<ClientSession>>, bool), ()>>;

    fn handle(&mut self, GameRemovePlayerMessage(pid): GameRemovePlayerMessage, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(
            Self::remove_player(self.id, pid)
            .map(|client, this, _| {
                Ok((client.map_err(|_| ())?, this.is_empty())) 
            })
        )
    }
}

impl Handler<GameNotifyPlayerMessage> for GameServer {
    type Result = ();

    fn handle(&mut self, msg: GameNotifyPlayerMessage, _ctx: &mut Self::Context) -> Self::Result {
        let client = self.clients.get(&msg.0).unwrap();
        client.do_send(msg.1);
    }
}

impl Handler<GameNotifyFactionMessage> for GameServer {
    type Result = ();

    fn handle(&mut self, msg: GameNotifyFactionMessage, ctx: &mut Self::Context) -> Self::Result {
        let gid = self.id;
        ctx.wait(wrap_future(async move {
            let res = Self::faction_broadcast(gid, msg.0, msg.1).await;
            if res.is_err() {
                println!("Faction broadcast failed : {:?}", res.err());
            }
        }));
    }
}

impl Handler<GameFleetTravelMessage> for GameServer {
    type Result = ();

    fn handle(&mut self, msg: GameFleetTravelMessage, ctx: &mut Self::Context) -> Self::Result {
        let gid = self.id;
        let state = state();
        let fleet = msg.fleet.clone();
        let player = fleet.player;
        let fid = fleet.id;
        ctx.wait(wrap_future(async move {
            Self::ws_broadcast(gid, protocol::Message::new(
                protocol::Action::FleetSailed,
                fleet,
                Some(player),
            )).await;
        }));
        // In this case, there is no battle, but a in-progress conquest
        // We update the conquest or cancel it depending on the remaining fleets
        let fleet = msg.fleet.clone();
        let system = msg.system.clone();
        ctx.wait(wrap_future(async move {
            if let Some(mut conquest) = Conquest::find_current_by_system(&system.id, &state.db_pool).await.unwrap() {
                conquest.remove_fleet(&system, &fleet, gid).await;
            }
        }));

        let datetime: DateTime<Utc> = msg.fleet.destination_arrival_date.unwrap().into();
        ctx.run_later(datetime.signed_duration_since(Utc::now()).to_std().unwrap(), move |this, ctx| {
            let gid = this.id;
            ctx.wait(wrap_future(async move {
                let res = process_fleet_arrival(gid, fid).await;
                if res.is_err() {
                    println!("Fleet arrival fail : {:?}", res.err());
                }
            }));
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
            move |this, ctx| (msg.callback)(this, ctx)
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
        let state = state();
        for (pid, c) in self.clients.iter() {
            state.add_client(&pid, c.clone());
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
    where F: FnMut(&mut GameServer, &mut <GameServer as Actor>::Context) -> Result<()> + 'static,
{
    ctx.run_interval(duration, move |this, ctx| {
        let result = closure(this, ctx).map_err(ServerError::from);
        if result.is_err() {
            println!("{:?}", result.err());
        }
    });
}
