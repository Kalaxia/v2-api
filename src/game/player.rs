use actix_web::{web, get, patch, post, HttpResponse};
use actix::Addr;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::{
    AppState,
    game::faction::FactionID,
    lib::{Result, auth},
    ws::client::ClientSession
};

#[derive(Serialize, Deserialize, Clone)]
pub struct PlayerData {
    pub username: String,
    pub faction: Option<FactionID>,
    pub ready: bool
}

pub struct Player {
    pub data: PlayerData,
    pub websocket: Option<Addr<ClientSession>>,
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, Hash, PartialEq, Eq)]
pub struct PlayerID(Uuid);

#[derive(Deserialize)]
pub struct PlayerUsername{
    pub username: String
}

#[derive(Deserialize)]
pub struct PlayerFaction{
    pub faction: FactionID
}

#[derive(Deserialize)]
pub struct PlayerReady{
    pub ready: bool
}

#[post("/login")]
pub async fn login(state:web::Data<AppState>) -> Result<auth::Claims> {
    let player = Player {
        data: PlayerData {
            username: String::from(""),
            faction: None,
            ready: false,
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

#[patch("/me/username")]
pub async fn update_username(state: web::Data<AppState>, data: web::Json<PlayerUsername>, claims: auth::Claims)
    -> Option<HttpResponse>
{
    let mut players = state.players.write().unwrap();
    players
        .get_mut(&claims.pid)
        .map(|p| {
            p.data.username = data.username.clone();
            HttpResponse::NoContent().finish()
        })
}

#[patch("/me/faction")]
pub async fn update_faction(state: web::Data<AppState>, data: web::Json<PlayerFaction>, claims: auth::Claims)
    -> Option<HttpResponse>
{
    let mut players = state.players.write().unwrap();
    players
        .get_mut(&claims.pid)
        .map(|p| {
            p.data.faction = Some(data.faction);
            HttpResponse::NoContent().finish()
        })
}

#[patch("/me/ready")]
pub async fn update_ready(state: web::Data<AppState>, data: web::Json<PlayerReady>, claims: auth::Claims)
    -> Option<HttpResponse>
{
    let mut players = state.players.write().unwrap();
    players
        .get_mut(&claims.pid)
        .map(|p| {
            if p.data.username.is_empty() || p.data.faction == None {
                panic!("username and faction must be set")
            }
            p.data.ready = data.ready;
            HttpResponse::NoContent().finish()
        })
}