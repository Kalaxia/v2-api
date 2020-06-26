use actix_web::{web, App, HttpServer};
use actix_web::middleware::Logger;
use std::collections::HashMap;
use std::sync::RwLock;
use std::env;
use env_logger;
#[cfg(feature="ssl-secure")]
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use sqlx::PgPool;

mod ws;
mod game;
mod lib;

use game::{
    fleet::ship,
    fleet::fleet,
    game as g,
    faction,
    player,
    lobby,
};
use lib::Result;

/// Global state of the game, containing everything we need to access from everywhere.
/// Each attribute is between a [`RwLock`](https://doc.rust-lang.org/std/sync/struct.RwLock.html)
pub struct AppState {
    db_pool: PgPool,
    clients: RwLock<HashMap<player::PlayerID, actix::Addr<ws::client::ClientSession>>>,
    lobbies: RwLock<HashMap<lobby::LobbyID, actix::Addr<lobby::LobbyServer>>>,
    games: RwLock<HashMap<g::GameID, actix::Addr<g::GameServer>>>,
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
    pub fn ws_broadcast(&self, message: ws::protocol::Message) {
        self.clients().iter().for_each(|(_, c)| c.do_send(message.clone()));
    }

    pub async fn clear_lobby(&self, lobby: lobby::Lobby, pid: player::PlayerID) -> lib::Result<()> {
        lobby::Lobby::remove(lobby.id, &self.db_pool).await?;
        self.ws_broadcast(ws::protocol::Message::new(
            ws::protocol::Action::LobbyRemoved,
            lobby,
            Some(pid),
        ));
        Ok(())
    }

    pub async fn clear_game(&self, gid: g::GameID) -> lib::Result<()> {
        g::Game::remove(gid.clone(), &self.db_pool).await?;
        self.games_mut().remove(&gid);
        Ok(())
    }

    pub fn add_client(&self, pid: &player::PlayerID, client: actix::Addr<ws::client::ClientSession>) {
        self.clients_mut().insert(pid.clone(), client);
    }

    pub fn retrieve_client(&self, pid: &player::PlayerID) -> actix::Addr<ws::client::ClientSession> {
        let mut clients = self.clients_mut();
        let client = clients.get(&pid).expect("Client not found").clone();
        clients.remove(&pid);
        client
    }

    pub fn remove_client(&self, pid: &player::PlayerID) {
        self.clients_mut().remove(pid);
    }

    res_access!{ games, games_mut : HashMap<g::GameID, actix::Addr<g::GameServer>> }
    res_access!{ lobbies, lobbies_mut : HashMap<lobby::LobbyID, actix::Addr<lobby::LobbyServer>> }
    res_access!{ clients, clients_mut : HashMap<player::PlayerID, actix::Addr<ws::client::ClientSession>> }
}

async fn generate_state() -> AppState {
    AppState {
        db_pool: create_pool().await.unwrap(),
        games: RwLock::new(HashMap::new()),
        lobbies: RwLock::new(HashMap::new()),
        clients: RwLock::new(HashMap::new()),
    }
}

// this function could be located in different module
fn config(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api")
        .service(
            web::scope("/factions")
            .service(faction::get_factions)
        )
        .service(
            web::scope("/games")
            .service(g::get_players)
            .service(
                web::scope("/{game_id}/systems")
                .service(
                    web::scope("/{system_id}/fleets")
                    .service(fleet::create_fleet)
                    .service(
                        web::scope("/{fleet_id}")
                        .service(fleet::travel)
                        .service(
                            web::scope("/ships")
                            .service(ship::add_ship)
                        )
                    )
                )
            )
        )
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
            .service(player::update_current_player)
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

async fn create_pool() -> Result<PgPool> {
    Ok(PgPool::new(&format!(
        "postgres://{}:{}@{}/{}",
        &get_env("POSTGRES_USER", "postgres"),
        &get_env("POSTGRES_PASSWORD", "root"),
        &get_env("POSTGRES_HOST", "localhost"),
        &get_env("POSTGRES_DB", "kalaxia_api")
    )).await?)
}

#[actix_rt::main]
#[cfg(feature="ssl-secure")]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();

    let key = get_env("SSL_PRIVATE_KEY", "../var/ssl/key.pem");
    let cert = get_env("SSL_CERTIFICATE", "../var/ssl/cert.pem");

    let mut ssl_config = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    ssl_config.set_private_key_file(key, SslFiletype::PEM).unwrap();
    ssl_config.set_certificate_chain_file(cert).unwrap();

    let state = web::Data::new(generate_state().await);

    HttpServer::new(move || App::new()
        .wrap(Logger::default())
        .app_data(state.clone()).configure(config))
        .bind_openssl(get_env("LISTENING_URL", "127.0.0.1:443"), ssl_config)?
        .run()
        .await
}

#[actix_rt::main]
#[cfg(not(feature="ssl-secure"))]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();
    
    let state = web::Data::new(generate_state().await);

    HttpServer::new(move || App::new()
        .wrap(Logger::default())
        .app_data(state.clone()).configure(config))
        .bind(get_env("LISTENING_URL", "127.0.0.1:80"))?
        .run()
        .await
}
