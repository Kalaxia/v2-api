use actix_web::{web, App, HttpServer};
use game::lobby;
use game::player;
use std::collections::HashMap;
use std::clone::Clone;
use std::sync::RwLock;
use std::env;
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

pub struct AppState {
    factions: RwLock<HashMap<FactionID, Faction>>,
    lobbies: RwLock<HashMap<LobbyID, Lobby>>,
    players: RwLock<HashMap<PlayerID, Player>>,
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
        let mut players = self.players.read().unwrap();
        let ofp = only_free_players.unwrap_or(false);
        players.iter().for_each(|(_, p)| {
            if (!ofp || (ofp && p.data.lobby == None)) && Some(p.data.id) != skip_id {
                p.websocket.as_ref().map(|ws| ws.do_send(message.clone()));
            }
        });
    }
}

fn generate_state() -> AppState {
    AppState {
        factions: RwLock::new(generate_factions()),
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
        )
        .service(
            web::scope("/players")
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
#[cfg(target_feature="ssl-secure")]
async fn main() -> std::io::Result<()> {
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder.set_private_key_file(get_env("SSL_PRIVATE_KEY", "../var/ssl/key.pem").unwrap(), SslFiletype::PEM).unwrap();
    builder.set_certificate_chain_file(env::var_os("SSL_CERTIFICATE", "../var/ssl/cert.pem").unwrap()).unwrap();

    HttpServer::new(move || App::new().app_data(web::Data::new(generate_state())).configure(config))
        .bind_openssl(get_env("LISTENING_URL", "127.0.0.1:80"), builder)?
        .run()
        .await
}

#[actix_rt::main]
#[cfg(not(target_feature="ssl-secure"))]
async fn main() -> std::io::Result<()> {
    HttpServer::new(move || App::new().app_data(web::Data::new(generate_state())).configure(config))
        .bind(get_env("LISTENING_URL", "127.0.0.1:80"))?
        .run()
        .await
}