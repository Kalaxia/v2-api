use actix_web::{web, get, patch, post, HttpResponse};
use actix::Addr;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;
use crate::{
    AppState,
    game::game::{GameID},
    game::lobby::{LobbyID, Lobby},
    game::faction::FactionID,
    lib::{Result, error::InternalError, auth},
    ws::protocol,
    ws::client::ClientSession
};

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq)]
pub struct PlayerData {
    pub id: PlayerID,
    pub username: String,
    pub game: Option<GameID>,
    pub lobby: Option<LobbyID>,
    pub faction: Option<FactionID>,
    pub ready: bool,
    pub wallet: usize,
}

#[derive(Clone)]
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
    pub faction_id: FactionID
}

#[derive(Deserialize)]
pub struct PlayerReady{
    pub ready: bool
}

impl Player {
    pub fn notify_update(data: PlayerData, players: &HashMap<PlayerID, Player>, lobby: &Lobby) {
        let id = data.id;
        lobby.ws_broadcast(&players, protocol::Message::new(
            protocol::Action::PlayerUpdate,
            data
        ), Some(&id))
    }

    pub fn spend(&mut self, amount: usize) -> Result<()> {
        if amount > self.data.wallet {
            return Err(InternalError::NotEnoughMoney)?;
        }
        self.data.wallet -= amount;
        Ok(())
    }
}

#[post("/login")]
pub async fn login(state:web::Data<AppState>) -> Result<auth::Claims> {
    let pid = PlayerID(Uuid::new_v4());
    let player = Player {
        data: PlayerData {
            id: pid.clone(),
            username: String::from(""),
            lobby: None,
            game: None,
            faction: None,
            ready: false,
            wallet: 0,
        },
        websocket: None,
    };

    let mut players = state.players_mut();
    players.insert(pid, player);
    
    Ok(auth::Claims { pid })
}

#[get("/count/")]
pub async fn get_nb_players(state:web::Data<AppState>)
    -> Option<HttpResponse>
{
    #[derive(Serialize)]
    struct PlayersCount {
        nb_players: usize
    }
    Some(HttpResponse::Ok().json(PlayersCount{
        nb_players: state.players.read().map_or(0, |players| players.len())
    }))
}

#[get("/me/")]
pub async fn get_current_player(state:web::Data<AppState>, claims: auth::Claims)
    -> Option<HttpResponse>
{
    let players = state.players();
    players
        .get(&claims.pid)
        .map(|p| HttpResponse::Ok().json(p.data.clone()))
}

#[patch("/me/username")]
pub async fn update_username(state: web::Data<AppState>, json_data: web::Json<PlayerUsername>, claims: auth::Claims)
    -> Option<HttpResponse>
{
    let mut players = state.players_mut();
    let data = players.get_mut(&claims.pid).map(|p| {
        p.data.username = json_data.username.clone();
        p.data.clone()
    })?;
    let lobbies = state.lobbies();
    let lobby = lobbies.get(&data.clone().lobby.unwrap()).unwrap();
    Player::notify_update(data.clone(), &players, lobby);
    drop(players);

    if lobby.creator == Some(data.id) {
        #[derive(Serialize, Clone)]
        struct LobbyName{
            id: LobbyID,
            name: String
        };
        state.ws_broadcast(protocol::Message::new(
            protocol::Action::LobbyNameUpdated,
            LobbyName{ id: lobby.id.clone(), name: data.username.clone() }
        ), Some(data.id), Some(true));
    }

    Some(HttpResponse::NoContent().finish())
}

#[patch("/me/faction")]
pub async fn update_faction(state: web::Data<AppState>, json_data: web::Json<PlayerFaction>, claims: auth::Claims)
    -> Option<HttpResponse>
{
    let factions = state.factions();
    let lobbies = state.lobbies();
    let mut players = state.players_mut();
    let data = players.get_mut(&claims.pid).map(|p| {
        if !factions.contains_key(&json_data.faction_id) {
            panic!("faction not found");
        }
        p.data.faction = Some(json_data.faction_id);
        p.data.clone()
    })?;
    Player::notify_update(data.clone(), &players, lobbies.get(&data.lobby.unwrap()).unwrap());
    Some(HttpResponse::NoContent().finish())
}

#[patch("/me/ready")]
pub async fn update_ready(state: web::Data<AppState>, claims: auth::Claims)
    -> Option<HttpResponse>
{
    let lobbies = state.lobbies();
    let mut players = state.players_mut();
    let data = players.get_mut(&claims.pid).map(|p| {
        p.data.ready = !p.data.ready;
        p.data.clone()
    })?;
    Player::notify_update(data.clone(), &players, lobbies.get(&data.lobby.unwrap()).unwrap());
    Some(HttpResponse::NoContent().finish())
}
