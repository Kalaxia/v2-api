use actix_web::{get, post, delete, web, HttpResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::lib::auth::Claims;
use crate::game::player::Player;
use crate::AppState;
use std::collections::HashMap;

#[derive(Copy, Clone, Serialize, Deserialize)]
enum LobbyStatus{
    Gathering,
    InProgress
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Lobby {
    id: Uuid,
    status: LobbyStatus,
    creator: Option<Player>,
    players: HashMap<Uuid, Player>
}

#[get("/")]
pub async fn get_lobbies(state: web::Data<AppState>) -> Option<HttpResponse> {
    Some(HttpResponse::Ok()
        .json(state.lobbies
            .read()
            .unwrap()
            .iter()
            .map(|(_, lobby)| lobby.clone())
            .collect::<Vec<Lobby>>()
        )
    )
}

#[get("/{id}")]
pub async fn get_lobby(info: web::Path<(Uuid,)>, state: web::Data<AppState>) -> Option<HttpResponse> {
    let lobbies = state.lobbies.read().unwrap();
    lobbies
        .get(&info.0)
        .map(| lobby | {
            HttpResponse::Ok().json(lobby)
        })
}

#[post("/")]
pub async fn create_lobby(state: web::Data<AppState>, claims: Claims) -> Option<HttpResponse> {
    let id = Uuid::new_v4();
    let mut lobbies = state.lobbies.write().unwrap();
    lobbies.insert(id, Lobby{
        id: id,
        status: LobbyStatus::Gathering,
        creator: Some(claims.player.clone()),
        players: [(claims.player.id, claims.player)].iter().cloned().collect::<HashMap<Uuid, Player>>(),
    });
    Some(HttpResponse::Created().json(lobbies.get(&id)))
}

#[post("/{id}/players/")]
pub async fn join_lobby(info: web::Path<(Uuid,)>,state: web::Data<AppState>, claims: Claims) -> Option<HttpResponse> {
    let mut lobbies = state.lobbies.write().unwrap();
    let lobby = lobbies.get_mut(&info.0).unwrap();
    lobby.players.insert(claims.player.id, claims.player.clone());
    Some(HttpResponse::NoContent().finish())
}

#[delete("/{id}/players/")]
pub async fn leave_lobby(info: web::Path<(Uuid,)>, state: web::Data<AppState>, claims: Claims) -> Option<HttpResponse> {
    let mut lobbies = state.lobbies.write().unwrap();
    let lobby = lobbies.get_mut(&info.0).unwrap();
    lobby.players.remove(&claims.player.id);
    if lobby.players.len() < 1 {
        lobbies.remove(&info.0);
    }
    Some(HttpResponse::NoContent().finish())
}