use actix_web::{delete, get, post, web, HttpResponse};
use actix::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::{
    lib::{
        Result,
        error::{InternalError},
        auth::Claims
    },
    game::game::{create_game},
    game::player::{PlayerID, Player},
    ws::{ client::ClientSession, protocol},
    AppState,
};
use std::sync::RwLock;
use std::collections::{HashMap, HashSet};
use sqlx::{PgPool, postgres::{PgRow, PgQueryAs}, FromRow, Error, QueryAs, Postgres};
use sqlx_core::row::Row;

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct LobbyID(pub Uuid);

impl From<LobbyID> for Uuid {
    fn from(lid: LobbyID) -> Self { lid.0 }
}

pub struct LobbyServer {
    pub id: LobbyID,
    pub clients: RwLock<HashMap<PlayerID, actix::Addr<ClientSession>>>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Lobby {
    pub id: LobbyID,
    pub owner: PlayerID,
}

impl<'a> FromRow<'a, PgRow<'a>> for Lobby {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        let id : Uuid = row.try_get("id")?;
        let owner_id : Uuid = row.try_get("owner_id")?;

        Ok(Lobby {
            id: LobbyID(id),
            owner: PlayerID(owner_id),
        })
    }
}

impl LobbyServer {
    pub fn ws_broadcast(&self, message: protocol::Message, skip_id: Option<PlayerID>) {
        let clients = self.clients.read().expect("Poisoned lock on lobby clients");
        for (id, client) in clients.iter() {
            if Some(*id) != skip_id && clients.contains_key(id) {
                client.do_send(message.clone());
            }
        }
    }
    
    pub fn is_empty(&self) -> bool {
        let clients = self.clients.read().expect("Poisoned lock on lobby clients");

        clients.len() == 0
    }

    pub fn remove_player(&mut self, pid: PlayerID) {
        let mut clients = self.clients.write().expect("Poisoned lock on lobby clients");
        // Remove the player from the lobby's list and notify all remaining players
        clients.remove(&pid.clone());
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::PlayerLeft,
            pid.clone()
        ), Some(pid.clone()));
    }
}

impl Lobby {
    pub async fn update_owner(&mut self, db_pool: &PgPool) {
        let players = Player::find_by_lobby(self.id, db_pool).await;
        self.owner = players.iter().next().unwrap().id.clone();
        Self::update(self.clone(), db_pool);
    }

    pub async fn find_all(db_pool: &PgPool) -> Vec<Self> {
        let lobbies: Vec<Self> = sqlx::query_as("SELECT * FROM lobby__lobbies")
            .fetch_all(db_pool).await.expect("Could not retrieve lobbies");
        lobbies
    }

    pub async fn find(lid: LobbyID, db_pool: &PgPool) -> Option<Self> {
        sqlx::query_as("SELECT * FROM lobby__lobbies WHERE id = $1")
            .bind(Uuid::from(lid))
            .fetch_one(db_pool).await.ok()
    }

    pub async fn create(l: Lobby, db_pool: &PgPool) {
        sqlx::query("INSERT INTO lobby__lobbies(id, owner_id) VALUES($1, $2)")
            .bind(Uuid::from(l.id))
            .bind(Uuid::from(l.owner))
            .execute(db_pool).await.expect("Could not create lobby");
    }

    pub async fn update(l: Lobby, db_pool: &PgPool) {
        sqlx::query("UPDATE lobby__lobbies SET owner_id = $1 WHERE id = $2")
            .bind(Uuid::from(l.id))
            .bind(Uuid::from(l.owner))
            .execute(db_pool).await.expect("Could not update lobby");
    }

    pub async fn remove(lid: LobbyID, db_pool: &PgPool) {
        sqlx::query("DELETE FROM lobby__lobbies WHERE id = $1")
            .bind(Uuid::from(lid))
            .execute(db_pool).await.expect("Could not delete lobby");
    }
}

impl Actor for LobbyServer {
    type Context = Context<Self>;
}

#[derive(actix::Message, Serialize, Clone)]
#[rtype(result="bool")]
pub struct LobbyRemovePlayerMessage(pub PlayerID);

#[derive(actix::Message)]
#[rtype(result="()")]
pub struct LobbyBroadcastMessage {
    pub message: protocol::Message,
    pub skip_id: Option<PlayerID>
}

impl Handler<LobbyRemovePlayerMessage> for LobbyServer {
    type Result = bool;

    fn handle(&mut self, LobbyRemovePlayerMessage(pid): LobbyRemovePlayerMessage, ctx: &mut Self::Context) -> Self::Result {
        self.remove_player(pid);
        if self.is_empty() {
            ctx.stop();
            ctx.terminate();
            return true;
        }
        false
    }
}

impl Handler<LobbyBroadcastMessage> for LobbyServer {
    type Result = ();

    fn handle(&mut self, msg: LobbyBroadcastMessage, _ctx: &mut Self::Context) -> Self::Result {
        self.ws_broadcast(msg.message, msg.skip_id);
    }
}

#[get("/")]
pub async fn get_lobbies(state: web::Data<AppState>) -> Option<HttpResponse> {
    #[derive(Serialize)]
    struct LobbyData{
        id: LobbyID,
        owner: Player,
        nb_players: i32
    }
    let lobbies = Lobby::find_all(&state.db_pool).await;
    let mut futures : Vec<(&Lobby, Option<Player>, i32)> = Vec::new();
    
    for lobby in lobbies.iter() {
        let (player, count) = futures::join!(
            Player::find(lobby.owner, &state.db_pool),
            Player::count_by_lobby(lobby.id, &state.db_pool)
        );
        futures.push((lobby, player, count));
    }

    //let joined : Vec<(&Lobby, Option<Player>, i32)> = futures::future::join_all(futures).await;
    let datas: Vec<LobbyData> = futures.into_iter()
        // Filter the lobbies with unexisting owner
        .filter_map(|(lobby, maybe_player, count)| {
            Some(LobbyData {
                id: lobby.id,
                owner: maybe_player?,
                nb_players: count,
            })
        })
        .collect();
    Some(HttpResponse::Ok().json(datas))
}

#[get("/{id}")]
pub async fn get_lobby(state: web::Data<AppState>, info: web::Path<(LobbyID,)>) -> Result<HttpResponse> {
    let lobby = Lobby::find(info.0, &state.db_pool).await.ok_or(InternalError::LobbyUnknown)?;

    #[derive(Serialize)]
    struct LobbyData{
        id: LobbyID,
        owner: Player,
        players: Vec<Player>,
    }

    Ok(HttpResponse::Ok().json(LobbyData{
        id: lobby.id,
        owner: Player::find(lobby.owner, &state.db_pool).await.ok_or(InternalError::PlayerUnknown)?,
        players: Player::find_by_lobby(lobby.id, &state.db_pool).await,
    }))
}

#[post("/")]
pub async fn create_lobby(state: web::Data<AppState>, claims: Claims) -> Result<HttpResponse> {
    // Get the requesting player identity
    let mut player = Player::find(claims.pid, &state.db_pool).await.ok_or(InternalError::PlayerUnknown)?;
    let mut lobby_servers = state.lobbies_mut();

    // If already in lobby, then error
    if player.lobby.is_some() {
        Err(InternalError::AlreadyInLobby)?
    }

    // Else, create a lobby
    let new_lobby = Lobby {
        id: LobbyID(Uuid::new_v4()),
        owner: player.id.clone(),
    };
    let lobby_server = LobbyServer{
        id: new_lobby.id.clone(),
        clients: RwLock::new(HashMap::new()),
    };
    lobby_servers.insert(lobby_server.id.clone(), lobby_server.start());
    // Put the player in the lobby
    player.lobby = Some(new_lobby.id.clone());
    Player::update(player.clone(), &state.db_pool);

    // Insert the lobby into the list
    Lobby::create(new_lobby.clone(), &state.db_pool);

    // Notify players for lobby creation
    state.ws_broadcast(protocol::Message::new(
        protocol::Action::LobbyCreated,
        new_lobby.clone(),
    ), Some(player.id), Some(true));

    Ok(HttpResponse::Created().json(new_lobby))
}

#[post("/{id}/launch/")]
pub async fn launch_game(state: web::Data<AppState>, claims:Claims, info: web::Path<(LobbyID,)>)
    -> Result<HttpResponse>
{
    let mut games = state.games_mut();

    let lobby = Lobby::find(info.0, &state.db_pool).await.ok_or(InternalError::LobbyUnknown)?;

    if lobby.owner != claims.pid.clone() {
        Err(InternalError::AccessDenied)?
    }
    let (game_id, game) = create_game(&lobby, state.clone()).await;
    games.insert(game_id.clone(), game.clone());

    state.ws_broadcast(protocol::Message::new(
        protocol::Action::LobbyLaunched,
        lobby.clone(),
    ), None, Some(true));

    Lobby::remove(lobby.id, &state.db_pool);

    Ok(HttpResponse::NoContent().finish())
}

#[delete("/{id}/players/")]
pub async fn leave_lobby(state:web::Data<AppState>, claims:Claims, info:web::Path<(LobbyID,)>)
    -> Result<HttpResponse>
{
    let mut lobby = Lobby::find(info.0, &state.db_pool).await.ok_or(InternalError::LobbyUnknown)?;
    let mut player = Player::find(claims.pid, &state.db_pool).await.ok_or(InternalError::PlayerUnknown)?;

    if player.lobby != Some(lobby.id) {
        Err(InternalError::NotInLobby)?
    }
    player.username = String::from("");
    player.faction = None;
    player.ready = false;
    player.lobby = None;

    let lobbies = state.lobbies();
    let lobby_server = lobbies.get(&lobby.id).ok_or(InternalError::LobbyUnknown)?;
    let is_empty = lobby_server.send(LobbyRemovePlayerMessage(player.id.clone())).await?;
    if is_empty {
        state.clear_lobby(lobby, player.id);
    } else if player.id == lobby.owner {
        lobby.update_owner(&state.db_pool).await;
        lobby_server.do_send(LobbyBroadcastMessage{
            message: protocol::Message::new(
                protocol::Action::LobbyOwnerUpdated,
                lobby.owner.clone()
            ),
            skip_id: None,
        });
    }
    Ok(HttpResponse::NoContent().finish())
}

#[post("/{id}/players/")]
pub async fn join_lobby(info: web::Path<(LobbyID,)>, state: web::Data<AppState>, claims: Claims)
    -> Result<HttpResponse>
{
    let lobby = Lobby::find(info.0, &state.db_pool).await.ok_or(InternalError::LobbyUnknown)?;
    let mut player = Player::find(claims.pid, &state.db_pool).await.ok_or(InternalError::PlayerUnknown)?;
    if player.lobby.is_some() {
        Err(InternalError::AlreadyInLobby)?
    }
    player.lobby = Some(lobby.id);
    Player::update(player.clone(), &state.db_pool);

    let message = protocol::Message::new(
        protocol::Action::PlayerJoined,
        player
    );
    let lobbies = state.lobbies();
    let lobby_server = lobbies.get(&lobby.id).ok_or(InternalError::LobbyUnknown)?;
    lobby_server.do_send(LobbyBroadcastMessage{
        message: message.clone(),
        skip_id: Some(claims.pid)
    });

    state.ws_broadcast(message, Some(claims.pid), Some(true));

    Ok(HttpResponse::NoContent().finish())
}
