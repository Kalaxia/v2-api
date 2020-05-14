use actix_web::{web, App, HttpServer};
use actix_web::middleware::Logger;
use game::lobby;
use game::player;
use game::game::{GameID, Game};
use std::collections::HashMap;
use std::clone::Clone;
use std::sync::RwLock;
use std::env;
use env_logger;
#[cfg(feature="ssl-secure")]
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};

mod ws;
mod game;
mod lib;

use game::{
    faction::{Faction, FactionID, generate_factions},
    player::{Player, PlayerID},
    lobby::{Lobby, LobbyID},
};

/// Global state of the game, containing everything we need to access from everywhere.
/// Each attribute is between a [`RwLock`](https://doc.rust-lang.org/std/sync/struct.RwLock.html)
pub struct AppState {
    factions: RwLock<HashMap<FactionID, Faction>>,
    games: RwLock<HashMap<GameID, actix::Addr<Game>>>,
    lobbies: RwLock<HashMap<LobbyID, Lobby>>,
    players: RwLock<HashMap<PlayerID, Player>>,
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
    pub fn ws_broadcast<T: 'static>(
        &self,
        message: &ws::protocol::Message<T>,
        skip_id: Option<PlayerID>,
        only_free_players: Option<bool>
    ) where
        T: Clone + Send + serde::Serialize
    {
        let players = self.players.read().unwrap();
        let ofp = only_free_players.unwrap_or(false);
        players.iter().for_each(|(_, p)| {
            if (!ofp || (ofp && p.data.lobby == None && p.data.game == None)) && Some(p.data.id) != skip_id {
                p.websocket.as_ref().map(|ws| ws.do_send(message.clone()));
            }
        });
    }

    res_access!{ factions, factions_mut : HashMap<FactionID, Faction> }
    res_access!{ games, games_mut : HashMap<GameID, actix::Addr<Game>> }
    res_access!{ lobbies, lobbies_mut : HashMap<LobbyID, Lobby> }
    res_access!{ players, players_mut : HashMap<PlayerID, Player> }
}

fn generate_state() -> AppState {
    AppState {
        factions: RwLock::new(generate_factions()),
        games: RwLock::new(HashMap::new()),
        lobbies: RwLock::new(HashMap::new()),
        players: RwLock::new(HashMap::new()),
    }
}

// this function could be located in different module
fn config(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api")
        .service(
            web::scope("/lobbies")
            .service(lobby::create_lobby)
            .service(lobby::get_lobbies)
            .service(lobby::get_lobby)
            .service(lobby::join_lobby)
            .service(lobby::leave_lobby)
            .service(lobby::launch_game)
        )
        .service(
            web::scope("/players")
            .service(player::get_nb_players)
            .service(player::get_current_player)
            .service(player::update_username)
            .service(player::update_faction)
            .service(player::update_ready)
        )
    )
    .service(player::login)
    .service(web::resource("/ws/").to(ws::client::entrypoint));
}

fn get_env(key: &str, default: &str) -> String {
    match env::var_os(key) {
        Some(val) => val.into_string().unwrap(),
        None => String::from(default)
    }
}

#[actix_rt::main]
#[cfg(feature="ssl-secure")]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();

    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder.set_private_key_file(get_env("SSL_PRIVATE_KEY", "../var/ssl/key.pem"), SslFiletype::PEM).unwrap();
    builder.set_certificate_chain_file(get_env("SSL_CERTIFICATE", "../var/ssl/cert.pem")).unwrap();

    let state = web::Data::new(generate_state());

    HttpServer::new(move || App::new()
        .wrap(Logger::default())
        .app_data(state.clone()).configure(config))
        .bind_openssl(get_env("LISTENING_URL", "127.0.0.1:443"), builder)?
        .run()
        .await
}

#[actix_rt::main]
#[cfg(not(feature="ssl-secure"))]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();
    
    let state = web::Data::new(generate_state());

    HttpServer::new(move || App::new()
        .wrap(Logger::default())
        .app_data(state.clone()).configure(config))
        .bind(get_env("LISTENING_URL", "127.0.0.1:80"))?
        .run()
        .await
}
