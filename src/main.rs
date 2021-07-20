#![allow(clippy::clone_on_copy)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::module_inception)]

#![warn(clippy::imprecise_flops)]
#![warn(clippy::suboptimal_flops)]
#![warn(clippy::clone_on_ref_ptr)]
#![warn(clippy::cognitive_complexity)]
#![warn(clippy::float_cmp_const)]
#![warn(clippy::implicit_hasher)]
#![warn(clippy::implicit_saturating_sub)]
#![warn(clippy::large_types_passed_by_value)]
#![warn(clippy::manual_ok_or)]
#![warn(clippy::missing_const_for_fn)]
#![warn(clippy::needless_pass_by_value)]
#![warn(clippy::non_ascii_literal)]
#![warn(clippy::trivially_copy_pass_by_ref)]
#![warn(clippy::type_repetition_in_bounds)]
#![warn(clippy::unreadable_literal)]
#![warn(clippy::unseparated_literal_suffix)]
#![warn(clippy::unused_self)]


use actix_web::{web, App, HttpServer};
use actix_web::middleware::Logger;
use std::collections::HashMap;
use std::sync::RwLock;
use std::env;
#[cfg(feature="ssl-secure")]
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use sqlx::PgPool;

extern crate gelf;

use gelf::{Logger as GelfLogger, TcpBackend, NullBackend, Message, Level};
use log::LevelFilter;

mod ws;
mod game;
mod lib;

use game::{
    fleet::fleet,
    fleet::travel,
    fleet::squadron as fleet_squadron,
    game::{
        game as g,
        server::{GameEndMessage, GameServer},
    },
    faction,
    player,
    lobby,
    system::building,
    system::system,
    ship::model,
    ship::queue,
    ship::squadron
};
use lib::Result;

/// Global state of the game, containing everything we need to access from everywhere.
/// Each attribute is between a [`RwLock`](https://doc.rust-lang.org/std/sync/struct.RwLock.html)
pub struct AppState {
    db_pool: PgPool,
    logger: GelfLogger,
    clients: RwLock<HashMap<player::PlayerID, actix::Addr<ws::client::ClientSession>>>,
    lobbies: RwLock<HashMap<lobby::LobbyID, actix::Addr<lobby::LobbyServer>>>,
    games: RwLock<HashMap<g::GameID, actix::Addr<GameServer>>>,
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
            let g = games.get(&game.id).unwrap().clone();
            games.remove(&game.id);
            g
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

    res_access!{ games, games_mut : HashMap<g::GameID, actix::Addr<GameServer>> }
    res_access!{ lobbies, lobbies_mut : HashMap<lobby::LobbyID, actix::Addr<lobby::LobbyServer>> }
    res_access!{ clients, clients_mut : HashMap<player::PlayerID, actix::Addr<ws::client::ClientSession>> }
}

async fn generate_state() -> AppState {
    AppState {
        db_pool: create_pool().await.unwrap(),
        logger: create_logger(),
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
            .service(g::leave_game)
            .service(
                web::scope("/{game_id}/factions")
                .service(
                    web::scope("/{faction_id}")
                    .service(player::get_faction_members)
                    .service(player::transfer_money)
                )
            )
            .service(
                web::scope("/{game_id}/systems")
                .service(system::get_systems)
                .service(
                    web::scope("/{system_id}/fleets")
                    .service(fleet::create_fleet)
                    .service(
                        web::scope("/{fleet_id}")
                        .service(fleet::donate)
                        .service(travel::travel)
                        .service(
                            web::scope("/squadrons")
                            .service(fleet_squadron::assign_ships)
                        )
                    )
                )
                .service(
                    web::scope("/{system_id}/squadrons")
                    .service(squadron::get_system_squadrons)
                )
                .service(
                    web::scope("/{system_id}/ship-queues")
                    .service(queue::add_ship_queue)
                    .service(queue::get_ship_queues)
                )
                .service(
                    web::scope("/{system_id}/buildings")
                    .service(building::get_system_buildings)
                    .service(building::create_building)
                )
            )
        )
        .service(
            web::scope("/lobbies")
            .service(lobby::create_lobby)
            .service(lobby::get_lobbies)
            .service(lobby::get_lobby)
            .service(lobby::join_lobby)
            .service(lobby::update_lobby_options)
            .service(lobby::leave_lobby)
            .service(lobby::launch_game)
        )
        .service(
            web::scope("/players")
            .service(player::get_nb_players)
            .service(player::get_current_player)
            .service(player::update_current_player)
        )
        .service(building::get_buildings_data)
        .service(g::get_game_constants)
        .service(model::get_ship_models)
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
    let result = PgPool::new(&format!(
        "postgres://{}:{}@{}/{}",
        &get_env("POSTGRES_USER", "kalaxia"),
        &get_env("POSTGRES_PASSWORD", "kalaxia"),
        &get_env("POSTGRES_HOST", "localhost"),
        &get_env("POSTGRES_DB", "kalaxia_api")
    )).await;
    if result.is_err() {
        panic!("Could not connect to database");
    }
    Ok(result?)
}

fn create_logger() -> GelfLogger {
    #[cfg(feature="graylog")]
    let backend = {
        println!("Graylog feature enabled");

        TcpBackend::new(&format!(
            "{}:{}",
            &get_env("GRAYLOG_HOST", "kalaxia_v2_graylog"),
            &get_env("GRAYLOG_PORT", "1514")
        )).expect("Failed to create TCP backend")
    };
    #[cfg(not(feature="graylog"))]
    let backend = NullBackend::new();

    let logger = GelfLogger::new(Box::new(backend))
        .expect("Failed to determine hostname");

    logger
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();

    let state = web::Data::new(generate_state().await);

    let mut server = HttpServer::new(move || App::new()
        .wrap(Logger::default())
        .app_data(state.clone()).configure(config));

    #[cfg(feature="ssl-secure")]
    {
        let key = get_env("SSL_PRIVATE_KEY", "../var/ssl/key.pem");
        let cert = get_env("SSL_CERTIFICATE", "../var/ssl/cert.pem");

        let mut ssl_config = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
        ssl_config.set_private_key_file(key, SslFiletype::PEM).unwrap();
        ssl_config.set_certificate_chain_file(cert).unwrap();

        server = server.bind_openssl(get_env("LISTENING_URL", "127.0.0.1:443"), ssl_config)?;
    }
    #[cfg(not(feature="ssl-secure"))]
    {
        server = server.bind(get_env("LISTENING_URL", "127.0.0.1:80"))?;
    }
    server.run().await
}