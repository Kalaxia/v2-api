use actix_web::{web, App, HttpServer};
use game::lobby;
use game::player;
use std::collections::HashMap;
use std::clone::Clone;
use std::sync::RwLock;
use uuid::Uuid;

mod ws;
mod game;
mod lib;

use game::player::{Player, PlayerID};

pub struct AppState {
    lobbies: RwLock<HashMap<Uuid, lobby::Lobby>>,
    players: RwLock<HashMap<PlayerID, Player>>,
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    // Start chat server actor
    let state = web::Data::new(AppState {
        lobbies: RwLock::new(HashMap::new()),
        players: RwLock::new(HashMap::new()),
    });

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .service(
                web::scope("/api")
                .service(
                    web::scope("/lobbies")
                    .service(lobby::create_lobby)
                    .service(lobby::get_lobbies)
                    .service(lobby::get_lobby)
                )
                .service(
                    web::scope("/players")
                    //.service(player::register)
                )
            )
            .service(player::login)
            .service(web::resource("/ws/").to(ws::client::entrypoint))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
