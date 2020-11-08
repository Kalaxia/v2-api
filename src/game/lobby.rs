use actix_web::{delete, get, patch, post, web, HttpResponse};
use actix::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::{
    lib::{
        Result,
        error::{ServerError, InternalError},
        auth::Claims
    },
    game::game::{create_game, GameOptionMapSize, GameOptionSpeed},
    game::player::{PlayerID, Player},
    ws::{ client::ClientSession, protocol},
    AppState,
};
use std::sync::{Arc, RwLock};
use std::collections::{HashMap};
use sqlx::{PgPool, postgres::{PgRow, PgQueryAs}, FromRow, Executor, Error, Postgres};
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
    pub game_speed: GameOptionSpeed, 
    pub map_size: GameOptionMapSize 
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LobbyOptionsPatch {
    pub map_size: Option<GameOptionMapSize>, 
    pub game_speed: Option<GameOptionSpeed>, 
}

impl<'a> FromRow<'a, PgRow<'a>> for Lobby {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        let id : Uuid = row.try_get("id")?;
        let owner_id : Uuid = row.try_get("owner_id")?;

        Ok(Lobby {
            id: LobbyID(id),
            owner: PlayerID(owner_id),
            game_speed: row.try_get("game_speed")?,
            map_size: row.try_get("map_size")?
        })
    }
}

impl LobbyServer {
    pub fn ws_broadcast(&self, message: protocol::Message) {
        let clients = self.clients.read().expect("Poisoned lock on lobby clients");
        for (_, c) in clients.iter() {
            c.do_send(message.clone());
        }
    }
    
    pub fn is_empty(&self) -> bool {
        let clients = self.clients.read().expect("Poisoned lock on lobby clients");

        clients.len() == 0
    }

    pub fn add_player(&mut self, pid: PlayerID, client: actix::Addr<ClientSession>) {
        let mut clients = self.clients.write().expect("Poisoned lock on lobby clients");

        clients.insert(pid, client);
    }

    pub fn remove_player(&mut self, pid: PlayerID) -> actix::Addr<ClientSession> {
        let mut clients = self.clients.write().expect("Poisoned lock on lobby clients");
        let client = clients.get(&pid).unwrap().clone();
        // Remove the player from the lobby's list and notify all remaining players
        clients.remove(&pid);
        drop(clients);
        self.ws_broadcast(protocol::Message::new(
            protocol::Action::PlayerLeft,
            pid.clone(),
            Some(pid.clone()),
        ));
        client
    }
}

impl Lobby {
    pub async fn update_owner(&mut self, db_pool: &PgPool) -> Result<()> {
        let players = Player::find_by_lobby(self.id, db_pool).await?;
        self.owner = players.iter().next().unwrap().id.clone();
        let mut tx = db_pool.begin().await?;
        self.update(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn find_all(db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM lobby__lobbies")
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find(lid: LobbyID, db_pool: &PgPool) -> Result<Self> {
        sqlx::query_as("SELECT * FROM lobby__lobbies WHERE id = $1")
            .bind(Uuid::from(lid))
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(InternalError::LobbyUnknown))
    }

    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO lobby__lobbies(id, owner_id, game_speed, map_size) VALUES($1, $2, $3, $4)")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.owner))
            .bind(self.game_speed)
            .bind(self.map_size)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn update<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("UPDATE lobby__lobbies SET owner_id = $2, game_speed = $3, map_size = $4 WHERE id = $1")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.owner))
            .bind(self.game_speed)
            .bind(self.map_size)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn remove<E>(&self, exec: &mut E) -> Result<u64> 
        where E: Executor<Database = Postgres>{
        sqlx::query("DELETE FROM lobby__lobbies WHERE id = $1")
            .bind(Uuid::from(self.id))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
}

impl Actor for LobbyServer {
    type Context = Context<Self>;
}

#[derive(actix::Message, Clone)]
#[rtype(result="()")]
pub struct LobbyAddClientMessage(pub PlayerID, pub actix::Addr<ClientSession>);

#[derive(actix::Message, Serialize, Clone)]
#[rtype(result="Arc<(actix::Addr<ClientSession>, bool)>")]
pub struct LobbyRemoveClientMessage(pub PlayerID);

#[derive(actix::Message, Clone)]
#[rtype(result="Arc<HashMap<PlayerID, actix::Addr<ClientSession>>>")]
pub struct LobbyGetClientsMessage();

impl Handler<LobbyAddClientMessage> for LobbyServer {
    type Result = ();

    fn handle(&mut self, LobbyAddClientMessage(pid, client): LobbyAddClientMessage, _ctx: &mut Self::Context) -> Self::Result {
        self.add_player(pid, client);
    }
}

impl Handler<LobbyRemoveClientMessage> for LobbyServer {
    type Result = Arc<(actix::Addr<ClientSession>, bool)>;

    fn handle(&mut self, LobbyRemoveClientMessage(pid): LobbyRemoveClientMessage, ctx: &mut Self::Context) -> Self::Result {
        let client = self.remove_player(pid);
        if self.is_empty() {
            ctx.stop();
            ctx.terminate();
            return Arc::new((client, true));
        }
        Arc::new((client, false))
    }
}

impl Handler<LobbyGetClientsMessage> for LobbyServer {
    type Result = Arc<HashMap<PlayerID, actix::Addr<ClientSession>>>;

    fn handle(&mut self, _msg: LobbyGetClientsMessage, _ctx: &mut Self::Context) -> Self::Result {
        let clients = self.clients.read().expect("Poisoned lock on lobby players");

        Arc::new(clients.clone())
    }
}

impl Handler<protocol::Message> for LobbyServer {
    type Result = ();

    fn handle(&mut self, msg: protocol::Message, _ctx: &mut Self::Context) -> Self::Result {
        self.ws_broadcast(msg);
    }
}

#[get("/")]
pub async fn get_lobbies(state: web::Data<AppState>) -> Result<HttpResponse> {
    #[derive(Serialize)]
    struct LobbyData{
        id: LobbyID,
        owner: Player,
        nb_players: i16
    }
    let lobbies = Lobby::find_all(&state.db_pool).await?;
    let mut futures : Vec<(&Lobby, Option<Player>, i16)> = Vec::new();
    
    for lobby in lobbies.iter() {
        let (player, count) = futures::join!(
            Player::find(lobby.owner, &state.db_pool),
            Player::count_by_lobby(lobby.id, &state.db_pool)
        );
        futures.push((lobby, player.ok(), count?));
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
    Ok(HttpResponse::Ok().json(datas))
}

#[get("/{id}")]
pub async fn get_lobby(state: web::Data<AppState>, info: web::Path<(LobbyID,)>) -> Result<HttpResponse> {
    let lobby = Lobby::find(info.0, &state.db_pool).await?;

    #[derive(Serialize)]
    struct LobbyData{
        id: LobbyID,
        owner: Player,
        players: Vec<Player>,
        game_speed: GameOptionSpeed,
        map_size: GameOptionMapSize
    }

    Ok(HttpResponse::Ok().json(LobbyData{
        id: lobby.id,
        owner: Player::find(lobby.owner, &state.db_pool).await?,
        players: Player::find_by_lobby(lobby.id, &state.db_pool).await?,
        game_speed: lobby.game_speed,
        map_size: lobby.map_size
    }))
}

#[post("/")]
pub async fn create_lobby(state: web::Data<AppState>, claims: Claims) -> Result<HttpResponse> {
    // Get the requesting player identity
    let mut player = Player::find(claims.pid, &state.db_pool).await?;
    let mut lobby_servers = state.lobbies_mut();

    // If already in lobby, then error
    if player.lobby.is_some() {
        Err(InternalError::AlreadyInLobby)?
    }

    // Else, create a lobby
    let new_lobby = Lobby {
        id: LobbyID(Uuid::new_v4()),
        owner: player.id.clone(),
        game_speed: GameOptionSpeed::Medium,
        map_size: GameOptionMapSize::Medium,
    };
    let lobby_server = LobbyServer{
        id: new_lobby.id.clone(),
        clients: RwLock::new(HashMap::new()),
    }.start();
    let client = state.retrieve_client(&claims.pid)?;
    lobby_server.do_send(LobbyAddClientMessage(player.id.clone(), client));
    lobby_servers.insert(new_lobby.id.clone(), lobby_server);
    // Insert the lobby into the list
    let mut tx = state.db_pool.begin().await?;
    new_lobby.insert(&mut tx).await?;
    // Put the player in the lobby
    player.lobby = Some(new_lobby.id.clone());
    player.update(&mut tx).await?;
    tx.commit().await?;
    // Notify players for lobby creation
    state.ws_broadcast(protocol::Message::new(
        protocol::Action::LobbyCreated,
        new_lobby.clone(),
        Some(player.id),
    ));

    Ok(HttpResponse::Created().json(new_lobby))
}

#[patch("/{id}/")]
pub async fn update_lobby_options(
    state: web::Data<AppState>,
    info: web::Path<(LobbyID,)>,
    data: web::Json<LobbyOptionsPatch>,
    claims: Claims
) -> Result<HttpResponse>
{
    let mut lobby = Lobby::find(info.0, &state.db_pool).await?;

    if lobby.owner != claims.pid.clone() {
        Err(InternalError::AccessDenied)?
    }
    println!("{:?}", data);
    lobby.game_speed = data.game_speed.clone().map_or(GameOptionSpeed::Medium, |gs| gs);
    lobby.map_size = data.map_size.clone().map_or(GameOptionMapSize::Medium, |ms| ms);

    println!("{:?}", lobby.game_speed.clone());
    println!("{:?}", lobby.map_size.clone());

    let mut tx = state.db_pool.begin().await?;
    lobby.update(&mut tx).await?;
    tx.commit().await?;

    let lobbies = state.lobbies();
    let lobby_server = lobbies.get(&lobby.id).ok_or(InternalError::LobbyUnknown)?;
    lobby_server.do_send(protocol::Message::new(
        protocol::Action::LobbyOptionsUpdated,
        data.clone(),
        Some(claims.pid),
    ));
    Ok(HttpResponse::NoContent().finish())
}

#[post("/{id}/launch/")]
pub async fn launch_game(state: web::Data<AppState>, claims:Claims, info: web::Path<(LobbyID,)>)
    -> Result<HttpResponse>
{
    let mut games = state.games_mut();

    let lobby = Lobby::find(info.0, &state.db_pool).await?;

    if lobby.owner != claims.pid.clone() {
        Err(InternalError::AccessDenied)?
    }
    let clients = Arc::try_unwrap({
        let lobbies = state.lobbies();
        let lobby_server = lobbies.get(&lobby.id).ok_or(InternalError::LobbyUnknown)?;
        lobby_server.send(LobbyGetClientsMessage{})
    }.await?).ok().unwrap();
    let (game_id, game) = create_game(&lobby, state.clone(), clients).await?;
    games.insert(game_id, game);

    state.ws_broadcast(protocol::Message::new(
        protocol::Action::LobbyLaunched,
        lobby.clone(),
        None,
    ));

    let mut tx = state.db_pool.begin().await?;
    lobby.remove(&mut tx).await?;
    tx.commit().await?;

    Ok(HttpResponse::NoContent().finish())
}

#[delete("/{id}/players/")]
pub async fn leave_lobby(state:web::Data<AppState>, claims:Claims, info:web::Path<(LobbyID,)>)
    -> Result<HttpResponse>
{
    let mut lobby = Lobby::find(info.0, &state.db_pool).await?;
    let mut player = Player::find(claims.pid, &state.db_pool).await?;

    if player.lobby != Some(lobby.id) {
        Err(InternalError::NotInLobby)?
    }
    player.reset(&state.db_pool).await?;

    let lobbies = state.lobbies();
    let lobby_server = lobbies.get(&lobby.id).expect("Lobby exists in DB but not in HashMap");
    let (client, is_empty) = Arc::try_unwrap(lobby_server.send(LobbyRemoveClientMessage(player.id.clone())).await?).ok().unwrap();
    state.add_client(&player.id, client.clone());
    if is_empty {
        state.clear_lobby(lobby, player.id).await?;
    } else if player.id == lobby.owner {
        lobby.update_owner(&state.db_pool).await?;
        lobby_server.do_send(protocol::Message::new(
            protocol::Action::LobbyOwnerUpdated,
            lobby.owner.clone(),
            None,
        ));
    }
    Ok(HttpResponse::NoContent().finish())
}

#[post("/{id}/players/")]
pub async fn join_lobby(info: web::Path<(LobbyID,)>, state: web::Data<AppState>, claims: Claims)
    -> Result<HttpResponse>
{
    let lobby = Lobby::find(info.0, &state.db_pool).await?;
    let mut player = Player::find(claims.pid, &state.db_pool).await?;
    if player.lobby.is_some() {
        Err(InternalError::AlreadyInLobby)?
    }
    player.lobby = Some(lobby.id);
    let mut tx = state.db_pool.begin().await?;
    player.update(&mut tx).await?;
    tx.commit().await?;

    let message = protocol::Message::new(
        protocol::Action::PlayerJoined,
        player,
        Some(claims.pid),
    );
    let client = state.retrieve_client(&claims.pid)?;
    let lobbies = state.lobbies();
    let lobby_server = lobbies.get(&lobby.id).ok_or(InternalError::LobbyUnknown)?;
    lobby_server.do_send(LobbyAddClientMessage(claims.pid.clone(), client));
    lobby_server.do_send(message.clone());

    state.ws_broadcast(message);

    Ok(HttpResponse::NoContent().finish())
}
