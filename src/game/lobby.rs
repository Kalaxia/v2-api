use actix_web::{get, post, web, HttpResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::game::player;
use crate::AppState;

#[derive(Serialize, Deserialize, Clone)]
pub struct Lobby {
    id: Uuid,
    creator: Option<player::Player>,
}

#[get("/")]
pub async fn get_lobbies(state: web::Data<AppState>) -> Option<HttpResponse> {
    Some(HttpResponse::Ok()
        .json(state.lobbies
            .lock()
            .unwrap()
            .iter()
            .map(|(_, lobby)| lobby.clone())
            .collect::<Vec<Lobby>>()
        )
    )
}

#[get("/{id}")]
pub async fn get_lobby(info: web::Path<(Uuid,)>, state: web::Data<AppState>) -> Option<HttpResponse> {
    let lobbies = state.lobbies.lock().unwrap();
    lobbies
        .get(&info.0)
        .map(| lobby | {
            HttpResponse::Ok().json(lobby)
        })
}

#[post("/")]
pub async fn create_lobby(state: web::Data<AppState>) -> Option<HttpResponse> {
    let id = Uuid::new_v4();
    let mut lobbies = state.lobbies.lock().unwrap();
    lobbies.insert(id, Lobby{
        id: id,
        creator: None
    });
    Some(HttpResponse::Ok().json(lobbies.get(&id)))
}