use actix_web::{web, get, post, HttpResponse};
use actix::Addr;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::{AppState, lib::{Result, auth}, ws::client::ClientSession};

#[derive(Serialize, Deserialize, Clone)]
pub struct PlayerData {
    pub username: String,
}

pub struct Player {
    pub data: PlayerData,
    pub websocket: Option<Addr<ClientSession>>,
}

#[derive(Serialize, Deserialize, Copy, Clone, Hash, PartialEq, Eq)]
pub struct PlayerID(Uuid);

#[post("/login")]
pub async fn login(state:web::Data<AppState>) -> Result<auth::Claims> {
    let player = Player {
        data: PlayerData {
            username: String::from(""),
        },
        websocket: None,
    };

    let mut players = state.players.write().unwrap();
    let pid = PlayerID(Uuid::new_v4());
    players.insert(pid, player);
    
    Ok(auth::Claims { pid })
}

#[get("/me/")]
pub async fn get_current_player(state:web::Data<AppState>, claims: auth::Claims)
    -> Option<HttpResponse>
{
    let players = state.players.read().unwrap();
    players
        .get(&claims.pid)
        .map(|p| HttpResponse::Ok().json(p.data.clone()))
}
