use actix_web::{delete, get, post, web, HttpResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::{
    lib::{
        Result,
        error::{InternalError},
        auth::Claims
    },
    game::game::{create_game},
    game::player,
    ws::protocol,
    AppState,
};
use std::collections::{HashMap, HashSet};

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct LobbyID(Uuid);

#[derive(Serialize, Deserialize, Clone)]
pub struct Lobby {
    pub id: LobbyID,
    pub owner: Option<player::PlayerID>,
    pub players: HashSet<player::PlayerID>,
}

impl Lobby {
    pub fn ws_broadcast(
        &self,
        players: &HashMap<player::PlayerID, player::Player>,
        message: protocol::Message,
        skip_id: Option<&player::PlayerID>
    ) {
        for (id, player) in players.iter() {
            if Some(id) != skip_id && self.players.contains(id) {
                player.websocket.as_ref().map(|ws| {
                    ws.do_send(message.clone());
                });
            }
        }
    }

    pub fn remove_player(
        &mut self,
        players: &HashMap<player::PlayerID, player::Player>,
        player_data: player::PlayerData,
    ) {
        // Remove the player from the lobby's list and notify all remaining players
        self.players.remove(&player_data.id.clone());
        self.ws_broadcast(&players, protocol::Message::new(
            protocol::Action::PlayerLeft,
            player_data.clone()
        ), Some(&player_data.id.clone()));
    }

    pub fn update_owner(&mut self, players: &HashMap<player::PlayerID, player::Player>) {
        self.owner = Some(self.players.iter().next().unwrap().clone());
        self.ws_broadcast(&players, protocol::Message::new(
            protocol::Action::LobbyOwnerUpdated,
            self.owner.clone()
        ), None);
    }
}

#[get("/")]
pub async fn get_lobbies(state: web::Data<AppState>) -> Option<HttpResponse> {
    #[derive(Serialize)]
    struct LobbyData{
        id: LobbyID,
        owner: Option<player::PlayerData>,
        nb_players: usize
    }

    let players = state.players();

    Some(HttpResponse::Ok()
        .json(state.lobbies()
            .iter()
            .map(|(_, lobby)| {
                let owner = lobby.owner.and_then(|pid| players.get(&pid));

                LobbyData {
                    id: lobby.id,
                    owner: owner.map(|p| p.data.clone()),
                    nb_players: lobby.players.len()
                }
            })
            .collect::<Vec<LobbyData>>()
        )
    )
}

#[get("/{id}")]
pub async fn get_lobby(state: web::Data<AppState>, info: web::Path<(LobbyID,)>) -> Option<HttpResponse> {
    let lobbies = state.lobbies();
    let players = state.players();

    let lobby = lobbies.get(&info.0)?;
    let owner = lobby.owner.and_then(|owner| players.get(&owner)).map(|p| p.data.clone());

    #[derive(Serialize)]
    struct LobbyData{
        id: LobbyID,
        owner: Option<player::PlayerData>,
        players: HashSet<player::PlayerData>,
    }

    let data = LobbyData{
        id: lobby.id,
        owner,
        players: lobby.players.iter().filter_map(|pid| Some(players.get(pid)?.data.clone())).collect(),
    };

    Some(HttpResponse::Ok().json(data))
}

#[post("/")]
pub async fn create_lobby(state: web::Data<AppState>, claims: Claims) -> Result<HttpResponse> {
    // Get the requesting player identity
    let pid = claims.pid;
    let mut players = state.players_mut();
    let player = players.get_mut(&pid).ok_or(InternalError::PlayerUnknown)?;

    // If already in lobby, then error
    if player.data.lobby.is_some() {
        Err(InternalError::AlreadyInLobby)?
    }

    // Else, create a lobby
    let id = LobbyID(Uuid::new_v4());
    let new_lobby = Lobby {
        id,
        owner: Some(pid),
        players: [pid].iter().copied().collect(),
    };

    // Put the player in the lobby
    player.data.lobby = Some(id);
    let data = player.data.clone();

    // Insert the lobby into the list
    let mut lobbies = state.lobbies_mut();
    lobbies.insert(id, new_lobby.clone());
    drop(players);

    // Notify plauers for lobby creation
    state.ws_broadcast(protocol::Message::new(
        protocol::Action::LobbyCreated,
        new_lobby.clone(),
    ), Some(data.id), Some(true));

    Ok(HttpResponse::Created().json(new_lobby))
}

#[post("/{id}/launch/")]
pub async fn launch_game(state: web::Data<AppState>, claims:Claims, info: web::Path<(LobbyID,)>)
    -> Result<HttpResponse>
{
    let mut players = state.players_mut();
    let mut lobbies = state.lobbies_mut();
    let mut games = state.games_mut();

    let lobby = lobbies.get(&info.0).ok_or(InternalError::LobbyUnknown)?;
    let lobby_id = lobby.id.clone();

    if lobby.owner != Some(claims.pid.clone()) {
        Err(InternalError::AccessDenied)?
    }
    let (game_id, game) = create_game(lobby, &mut players);
    games.insert(game_id.clone(), game.clone());
    // Avoid deadlock in state broadcast
    drop(players);

    state.ws_broadcast(protocol::Message::new(
        protocol::Action::LobbyLaunched,
        lobby.clone(),
    ), None, Some(true));

    // Clear the lobby
    drop(lobby);
    lobbies.remove(&lobby_id);

    Ok(HttpResponse::NoContent().finish())
}

#[delete("/{id}/players/")]
pub async fn leave_lobby(state:web::Data<AppState>, claims:Claims, info:web::Path<(LobbyID,)>)
    -> Result<HttpResponse>
{
    let mut players = state.players_mut();
    let mut lobbies = state.lobbies_mut();

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

    lobby.remove_player(&players, data.clone());

    if lobby.players.is_empty() {
        let lobby_to_remove = lobby.clone();
        drop(players);
        drop(lobbies);
        state.clear_lobby(lobby_to_remove, data.id);
    } else if Some(data.id) == lobby.owner {
        lobby.update_owner(&players);
    }
    Ok(HttpResponse::NoContent().finish())
}

#[post("/{id}/players/")]
pub async fn join_lobby(info: web::Path<(LobbyID,)>, state: web::Data<AppState>, claims: Claims)
    -> Result<HttpResponse>
{
    let mut lobbies = state.lobbies_mut();
    let mut players = state.players_mut();

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
    let message = protocol::Message::new(
        protocol::Action::PlayerJoined,
        data
    );
    lobby.ws_broadcast(&players, message.clone(), Some(&claims.pid));
    drop(players);

    state.ws_broadcast(message, Some(claims.pid), Some(true));

    Ok(HttpResponse::NoContent().finish())
}
