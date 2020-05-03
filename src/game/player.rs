use actix_web::{web, get, patch, post, HttpResponse};
use actix::Addr;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;
use crate::{
    AppState,
    game::lobby::{LobbyID, Lobby},
    game::faction::FactionID,
    lib::{Result, auth},
    ws::protocol,
    ws::client::ClientSession
};

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq)]
pub struct PlayerData {
    pub id: PlayerID,
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

impl Player {
    pub fn notify_update(data: PlayerData, players: &HashMap<PlayerID, Player>, lobbies: &HashMap<LobbyID, Lobby>) {
        lobbies.iter()
            .find(|(_, l)| l.has_player(data.id))
            .map(|(_, l)| {
                let id = data.id;
                l.ws_broadcast(&players, &protocol::Message::<PlayerData>{
                    action: protocol::Action::PlayerUpdate,
                    data
                }, Some(&id))
            });
    }
}

#[post("/login")]
pub async fn login(state:web::Data<AppState>) -> Result<auth::Claims> {
    let pid = PlayerID(Uuid::new_v4());
    let player = Player {
        data: PlayerData {
            id: pid.clone(),
            username: String::from(""),
            faction: None,
            ready: false,
        },
        websocket: None,
    };

    let mut players = state.players.write().unwrap();
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
pub async fn update_username(state: web::Data<AppState>, json_data: web::Json<PlayerUsername>, claims: auth::Claims)
    -> Option<HttpResponse>
{
    let lobbies = state.lobbies.read().unwrap();
    let mut players = state.players.write().unwrap();
    let data = players.get_mut(&claims.pid).map(|p| {
        p.data.username = json_data.username.clone();
        p.data.clone()
    })?;
    Player::notify_update(data, &players, &lobbies);
    Some(HttpResponse::NoContent().finish())
}

#[patch("/me/faction")]
pub async fn update_faction(state: web::Data<AppState>, json_data: web::Json<PlayerFaction>, claims: auth::Claims)
    -> Option<HttpResponse>
{
    let lobbies = state.lobbies.read().unwrap();
    let mut players = state.players.write().unwrap();
    let data = players.get_mut(&claims.pid).map(|p| {
        p.data.faction = Some(json_data.faction);
        p.data.clone()
    })?;
    Player::notify_update(data, &players, &lobbies);
    Some(HttpResponse::NoContent().finish())
}

#[patch("/me/ready")]
pub async fn update_ready(state: web::Data<AppState>, json_data: web::Json<PlayerReady>, claims: auth::Claims)
    -> Option<HttpResponse>
{
    let lobbies = state.lobbies.read().unwrap();
    let mut players = state.players.write().unwrap();
    let data = players.get_mut(&claims.pid).map(|p| {
        p.data.ready = json_data.ready;
        p.data.clone()
    })?;
    Player::notify_update(data, &players, &lobbies);
    Some(HttpResponse::NoContent().finish())
}