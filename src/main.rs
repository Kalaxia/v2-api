use actix_web::{web, App, HttpServer};
use game::lobby;
use game::player;
use std::collections::HashMap;
use std::clone::Clone;
use std::sync::RwLock;

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

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    // Start chat server actor
    let state = web::Data::new(AppState {
        factions: RwLock::new(generate_factions()),
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
            .service(web::resource("/ws/").to(ws::client::entrypoint))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
