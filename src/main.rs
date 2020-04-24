use actix_web::{web, App, HttpServer};
use game::lobby;
use game::player;
use std::collections::HashMap;
use uuid::Uuid;
use actix::Actor;
use std::clone::Clone;
use std::sync::RwLock;

mod ws;
mod game;
mod lib;

pub struct AppState {
    lobbies: RwLock<HashMap<Uuid, lobby::Lobby>>,
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    // Start chat server actor
    let server = ws::server::LobbyWebsocket::default().start();
    let state = web::Data::new(AppState {
        lobbies: RwLock::new(HashMap::new()),
    });

    HttpServer::new(move || {
        App::new()
            .data(server.clone())
            .app_data(state.clone())
            .service(
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
                )
            )
            .service(player::login)
            .service(web::resource("/ws/").to(ws::client::entrypoint))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
