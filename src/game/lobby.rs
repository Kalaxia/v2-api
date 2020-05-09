use actix_web::{delete, get, post, web, HttpResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::{
    lib::{
        Result,
        error::{InternalError},
        auth::Claims
    },
    game::player,
    ws::protocol,
    AppState,
};
use std::collections::{HashMap, HashSet};

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct LobbyID(Uuid);

#[derive(Copy, Clone, Serialize, Deserialize)]
pub enum LobbyStatus{
    Gathering,
    InProgress,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Lobby {
    pub id: LobbyID,
    pub status: LobbyStatus,
    pub creator: Option<player::PlayerID>,
    pub players: HashSet<player::PlayerID>,
}

impl Lobby {
    pub fn ws_broadcast<T: 'static>(
        &self,
        players: &HashMap<player::PlayerID, player::Player>,
        message: &protocol::Message<T>,
        skip_id: Option<&player::PlayerID>
    ) where
        T: Clone + Send + Serialize
    {
        for (id, player) in players.iter() {
            if Some(id) != skip_id && self.players.contains(id) {
                player.websocket.as_ref().map(|ws| {
                    ws.do_send(message.clone());
                });
            }
        }
    }
}

#[get("/")]
pub async fn get_lobbies(state: web::Data<AppState>) -> Option<HttpResponse> {
    #[derive(Serialize)]
    struct LobbyData{
        id: LobbyID,
        status: LobbyStatus,
        creator: Option<player::PlayerData>,
        nb_players: usize
    }

    let players = state.players.read().expect("Players RwLock poisoned");

    Some(HttpResponse::Ok()
        .json(state.lobbies
            .read()
            .expect("Lobbies RwLock poisoned")
            .iter()
            .map(|(_, lobby)| {
                let creator = lobby.creator.and_then(|pid| players.get(&pid));

                LobbyData {
                    id: lobby.id,
                    status: lobby.status,
                    creator: creator.map(|p| p.data.clone()),
                    nb_players: lobby.players.len()
                }
            })
            .collect::<Vec<LobbyData>>()
        )
    )
}

#[get("/{id}")]
pub async fn get_lobby(state: web::Data<AppState>, info: web::Path<(LobbyID,)>) -> Option<HttpResponse> {
    let lobbies = state.lobbies.read().expect("Lobbies RwLock poisoned");
    let players = state.players.read().expect("Players RwLock poisoned");

    let lobby = lobbies.get(&info.0)?;
    let creator = lobby.creator.and_then(|creator| players.get(&creator)).map(|p| p.data.clone());

    #[derive(Serialize)]
    struct LobbyData{
        id: LobbyID,
        status: LobbyStatus,
        creator: Option<player::PlayerData>,
        players: HashSet<player::PlayerData>,
    }

    let data = LobbyData{
        id: lobby.id,
        status: lobby.status,
        creator,
        players: lobby.players.iter().filter_map(|pid| Some(players.get(pid)?.data.clone())).collect(),
    };

    Some(HttpResponse::Ok().json(data))
}

#[post("/")]
pub async fn create_lobby(state: web::Data<AppState>, claims: Claims) -> Result<HttpResponse> {
    // Get the requesting player identity
    let pid = claims.pid;
    let mut players = state.players.write().expect("Players RwLock poisoned");
    let player = players.get_mut(&pid).ok_or(InternalError::PlayerUnknown)?;

    // If already in lobby, then error
    if player.data.lobby.is_some() {
        Err(InternalError::AlreadyInLobby)?
    }

    // Else, create a lobby
    let id = LobbyID(Uuid::new_v4());
    let new_lobby = Lobby {
        id,
        status: LobbyStatus::Gathering,
        creator: Some(pid),
        players: [pid].iter().copied().collect(),
    };

    // Put the player in the lobby
    player.data.lobby = Some(id);
    let data = player.data.clone();

    // Insert the lobby into the list
    let mut lobbies = state.lobbies.write().expect("Lobbies RwLock poisoned");
    lobbies.insert(id, new_lobby.clone());
    drop(players);

    // Notify plauers for lobby creation
    state.ws_broadcast(&protocol::Message::<Lobby>{
        action: protocol::Action::LobbyCreated,
        data: new_lobby.clone(),
    }, Some(data.id), Some(true));

    Ok(HttpResponse::Created().json(new_lobby))
}

#[delete("/{id}/players/")]
pub async fn leave_lobby(state:web::Data<AppState>, claims:Claims, info:web::Path<(LobbyID,)>)
    -> Result<HttpResponse>
{
    let mut players = state.players.write().expect("Players RwLock poisoned");
    let mut lobbies = state.lobbies.write().expect("Lobbies RwLock poisoned");

    let lobby = lobbies.get_mut(&info.0).ok_or(InternalError::LobbyUnknown)?;

    // Modify the player's shared data and return the new data
    let data = players
        .get_mut(&claims.pid)
        .ok_or(InternalError::PlayerUnknown)
        .and_then(|p| {
            if p.data.lobby != Some(lobby.id) {
                return Err(InternalError::NotInLobby)
            }

            p.data.username = String::from("");
            p.data.faction = None;
            p.data.ready = false;
            p.data.lobby = None;
            Ok(p.data.clone())
        })?;

    // Remove the player from the lobby's list and notify all remaining players
    lobby.players.remove(&claims.pid);
    lobby.ws_broadcast(&players, &protocol::Message::<player::PlayerData>{
        action: protocol::Action::PlayerLeft,
        data
    }, Some(&claims.pid));
    drop(players);

    // If it was the last player, remove the lobby and notify all players a lobby was closed
    if lobby.players.is_empty() {
        state.ws_broadcast(&protocol::Message::<Lobby>{
            action: protocol::Action::LobbyRemoved,
            data: lobby.clone(),
        }, Some(claims.pid), Some(true));
        lobbies.remove(&info.0);
    }

    Ok(HttpResponse::Ok().finish())
}

#[post("/{id}/players/")]
pub async fn join_lobby(info: web::Path<(LobbyID,)>, state: web::Data<AppState>, claims: Claims)
    -> Result<HttpResponse>
{
    let mut lobbies = state.lobbies.write().expect("Lobbies RwLock poisoned");
    let mut players = state.players.write().expect("Players RwLock poisoned");

    let lobby = lobbies.get_mut(&info.0).ok_or(InternalError::LobbyUnknown)?;

    let data = players
        .get_mut(&claims.pid)
        .ok_or(InternalError::PlayerUnknown)
        .and_then(|p| {
            if p.data.lobby.is_some() {
                return Err(InternalError::AlreadyInLobby)
            }
            p.data.lobby = Some(lobby.id);
            Ok(p.data.clone())
        })?;

    lobby.players.insert(claims.pid);
    let message = &protocol::Message::<player::PlayerData>{
        action: protocol::Action::PlayerJoined,
        data
    };
    lobby.ws_broadcast(&players, message, Some(&claims.pid));
    drop(players);

    state.ws_broadcast(message, Some(claims.pid), Some(true));

    Ok(HttpResponse::NoContent().finish())
}
