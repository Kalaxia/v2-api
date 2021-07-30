use crate::{
    lib::{self, Result, sync::SyncOnceCell},
    ws::{self, protocol},
    game::{
        player,
        lobby,
        game::{game as g, server::{GameEndMessage, GameServer}},
    },
};
use std::sync::RwLock;
use std::collections::HashMap;
use std::future::{Ready, ready};
use sqlx::PgPool;
use gelf::Logger as GelfLogger;
use actix_web::{HttpRequest, FromRequest, dev::Payload};

/// Global state of the game, containing everything we need to access from everywhere.
/// Each attribute is between a [`RwLock`](https://doc.rust-lang.org/std/sync/struct.RwLock.html)
pub struct AppState {
    pub db_pool: PgPool,
    pub logger: Option<GelfLogger>,
    clients: RwLock<HashMap<player::PlayerID, actix::Addr<ws::client::ClientSession>>>,
    lobbies: RwLock<HashMap<lobby::LobbyID, actix::Addr<lobby::LobbyServer>>>,
    games: RwLock<HashMap<g::GameID, actix::Addr<GameServer>>>,
    missing_messages: RwLock<HashMap<player::PlayerID, Vec<protocol::Message>>>,
}

macro_rules! res_access {
    { $name:ident , $name_mut:ident : $t:ty } => {
        pub fn $name(&self) -> std::sync::RwLockReadGuard<$t> {
            self.$name.read().expect(stringify!("AppState::", $name, "() RwLock poisoned"))
        }
        pub fn $name_mut(&self) -> std::sync::RwLockWriteGuard<$t> {
            self.$name.write().expect(stringify!("AppState::", $name_mut, "() RwLock poisoned"))
        }
    };
}

impl AppState {
    pub fn new(db_pool: PgPool, logger: Option<GelfLogger>) -> Self {
        Self {
            db_pool,
            logger,
            games: RwLock::new(HashMap::new()),
            lobbies: RwLock::new(HashMap::new()),
            clients: RwLock::new(HashMap::new()),
            missing_messages: RwLock::new(HashMap::new()),
        }
    }

    pub fn ws_broadcast(&self, message: &ws::protocol::Message) {
        self.clients().iter().for_each(|(_, c)| c.do_send(message.clone()));
    }

    pub async fn clear_lobby(&self, lobby: lobby::Lobby, pid: player::PlayerID) -> lib::Result<()> {
        let mut tx = self.db_pool.begin().await?;
        lobby.remove(&mut tx).await?;
        tx.commit().await?;
        self.ws_broadcast(&ws::protocol::Message::new(
            ws::protocol::Action::LobbyRemoved,
            lobby,
            Some(pid),
        ));
        Ok(())
    }

    pub async fn clear_game(&self, game: &g::Game) -> lib::Result<()> {
        let game_server = {
            let mut games = self.games_mut();
            games.remove(&game.id).unwrap()
        };
        game_server.do_send(GameEndMessage{});
        let mut tx = self.db_pool.begin().await?;
        game.remove(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub fn add_client(&self, pid: &player::PlayerID, client: actix::Addr<ws::client::ClientSession>) {
        self.clients_mut().insert(pid.clone(), client);
    }

    #[allow(clippy::or_fun_call)]
    pub fn retrieve_client(&self, pid: &player::PlayerID) -> Result<actix::Addr<ws::client::ClientSession>> {
        let mut clients = self.clients_mut();
        clients.remove_entry(&pid)
            .ok_or(lib::error::InternalError::PlayerUnknown.into())
            .map(|t| t.1)
    }

    pub fn remove_client(&self, pid: &player::PlayerID) {
        self.clients_mut().remove(pid);
    }

    pub fn ws_send(&self, pid: &player::PlayerID, message: &protocol::Message) {
        let msg = message.clone();
        if let Some(client) = self.clients().get(pid) {
            client.do_send(msg);
        } else {
            self.missing_messages_mut().entry(*pid)
                .or_default()
                .push(msg)
        }
    }

    res_access!{ games, games_mut : HashMap<g::GameID, actix::Addr<GameServer>> }
    res_access!{ lobbies, lobbies_mut : HashMap<lobby::LobbyID, actix::Addr<lobby::LobbyServer>> }
    res_access!{ clients, clients_mut : HashMap<player::PlayerID, actix::Addr<ws::client::ClientSession>> }
    res_access!{ missing_messages, missing_messages_mut : HashMap<player::PlayerID, Vec<protocol::Message>> }
}

static STATE : SyncOnceCell<AppState> = SyncOnceCell::new();

pub fn init(state: AppState) {
    STATE.set(state)
}

pub fn state() -> & 'static AppState {
    STATE.get().expect("Global state was not initialized")
}

impl FromRequest for & 'static AppState {
    type Error = ();
    type Future = Ready<std::result::Result<& 'static AppState, ()>>;
    type Config = ();

    fn from_request(_req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        ready(Ok(state()))
    }
}
