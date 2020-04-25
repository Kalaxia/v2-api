use actix_web::{delete, get, post, web, HttpResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::{
    lib::auth::Claims,
    game::player,
    ws::protocol::LobbyMessage,
    AppState,
};
use std::collections::HashSet;


#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Debug)]
pub struct LobbyID(Uuid);

#[derive(Copy, Clone, Serialize, Deserialize)]
enum LobbyStatus{
    Gathering,
    InProgress,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Lobby {
    status: LobbyStatus,
    creator: Option<player::PlayerID>,
    players: HashSet<player::PlayerID>,
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
pub async fn get_lobby(info: web::Path<(LobbyID,)>, state: web::Data<AppState>) -> Option<HttpResponse> {
    let lobbies = state.lobbies.read().unwrap();
    lobbies
        .get(&info.0)
        .map(| lobby | {
            HttpResponse::Ok().json(lobby)
        })
}

#[post("/")]
pub async fn create_lobby(state: web::Data<AppState>, claims: Claims) -> Option<HttpResponse> {
    let id = LobbyID(Uuid::new_v4());
    let mut lobbies = state.lobbies.write().unwrap();
    lobbies.insert(id.clone(), Lobby {
        status: LobbyStatus::Gathering,
        creator: Some(claims.pid.clone()),
        players: [claims.pid].iter().cloned().collect(),
    });

    let players = state.players.read().unwrap();
    for (_, player::Player { websocket, .. }) in players.iter() {
        websocket.as_ref().map(|ws| {
            ws.do_send(LobbyMessage::LobbyCreated);
        });
    }

    Some(HttpResponse::Created().json(lobbies.get(&id)))
}

#[delete("/{id}/players/")]
pub async fn leave_lobby(state:web::Data<AppState>, claims:Claims, info:web::Path<(LobbyID,)>)
    -> Option<HttpResponse>
{
    let mut remove_lobby = false;
    let mut lobbies = state.lobbies.write().unwrap();
    lobbies
        .get_mut(&info.0)
        .map(|lobby| {
            lobby.players.remove(&claims.pid);
            remove_lobby = lobby.players.is_empty();

            let players = state.players.read().unwrap();
            for (id, player::Player { websocket, ..}) in players.iter() {
                if *id != claims.pid && lobby.players.contains(id) {
                    websocket.as_ref().map(|ws| {
                        ws.do_send(LobbyMessage::PlayerDisconnected);
                    });
                }
            }
        })?;

    if remove_lobby {
        lobbies.remove(&info.0);
    }

    Some(HttpResponse::Ok().finish())
}

#[post("/{id}/players/")]
pub async fn join_lobby(info: web::Path<(LobbyID,)>, state: web::Data<AppState>, claims: Claims)
    -> Option<HttpResponse>
{
    let mut lobbies = state.lobbies.write().unwrap();
    let lobby = lobbies.get_mut(&info.0)?;

    lobby.players.insert(claims.pid);

    Some(HttpResponse::NoContent().finish())
}


