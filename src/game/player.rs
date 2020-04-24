use actix_web::{web, post};
use actix::Addr;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::{AppState, lib::{Result, auth}, ws::client::ClientSession};

pub struct Player {
    pub username: String,
    pub websocket: Option<Addr<ClientSession>>,
}

#[derive(Serialize, Deserialize, Copy, Clone, Hash, PartialEq, Eq)]
pub struct PlayerID(Uuid);

#[post("/login")]
pub async fn login(state:web::Data<AppState>) -> Result<auth::Claims> {
    let player = Player {
        username: String::from(""),
        websocket: None,
    };

    let mut players = state.players.write().unwrap();
    let pid = PlayerID(Uuid::new_v4());
    players.insert(pid, player);
    
    Ok(auth::Claims { pid })
}
