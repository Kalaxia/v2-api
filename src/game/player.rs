use actix_web::{web, get, patch, post, HttpResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use sqlx::{PgPool, postgres::{PgRow, PgQueryAs}, Executor, FromRow, Error, Postgres};
use sqlx_core::row::Row;
use crate::{
    AppState,
    game::game::{
        game::{GameID, GAME_START_WALLET},
        server::GameNotifyPlayerMessage,
    },
    game::lobby::{LobbyID, Lobby},
    game::faction::FactionID,
    game::system::system::SystemID,
    lib::{Result, error::{InternalError, ServerError}, auth},
    ws::protocol,
};

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq)]
pub struct Player {
    pub id: PlayerID,
    pub username: String,
    pub game: Option<GameID>,
    pub lobby: Option<LobbyID>,
    pub faction: Option<FactionID>,
    pub ready: bool,
    pub wallet: usize,
    pub is_connected: bool,
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, Hash, PartialEq, Eq)]
pub struct PlayerID(pub Uuid);

#[derive(Deserialize)]
pub struct PlayerUpdateData{
    pub username: String,
    pub faction_id: Option<FactionID>,
    pub is_ready: bool,
}

#[derive(Deserialize)]
pub struct PlayerMoneyTransferRequest{
    pub amount: usize
}

impl From<PlayerID> for Uuid {
    fn from(pid: PlayerID) -> Self { pid.0 }
}

impl<'a> FromRow<'a, PgRow<'a>> for Player {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(Player {
            id: row.try_get::<Uuid, _>("id").ok().map(PlayerID).unwrap(),
            username: row.try_get("username")?,
            faction: row.try_get::<i32, _>("faction_id").ok().map(|id| FactionID(id as u8)),
            game: row.try_get::<Uuid, _>("game_id").ok().map(GameID),
            lobby: row.try_get::<Uuid, _>("lobby_id").ok().map(LobbyID),
            wallet: row.try_get::<i32, _>("wallet").map(|w| w as usize)?,
            ready: row.try_get("is_ready")?,
            is_connected: row.try_get("is_connected")?,
        })
    }
}

impl Player {
    pub fn spend(&mut self, amount: usize) -> Result<()> {
        if amount > self.wallet {
            return Err(InternalError::NotEnoughMoney.into());
        }
        self.wallet -= amount;
        Ok(())
    }

    pub async fn reset(&mut self, db_pool: &PgPool) -> Result<()> {
        self.username = String::from("");
        self.faction = None;
        self.ready = false;
        self.lobby = None;
        self.game = None;
        let mut tx = db_pool.begin().await?;
        self.update(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn find(pid: PlayerID, db_pool: &PgPool) -> Result<Self> {
        sqlx::query_as("SELECT * FROM player__players WHERE id = $1")
            .bind(Uuid::from(pid))
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(InternalError::PlayerUnknown))
    }

    pub async fn find_system_owner(sid: SystemID, db_pool: &PgPool) -> Result<Self> {
        sqlx::query_as("SELECT p.* FROM map__systems s INNER JOIN player__players p ON p.id = s.player_id WHERE s.id = $1")
            .bind(Uuid::from(sid))
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(InternalError::PlayerUnknown))
    }

    pub async fn find_by_ids(ids: Vec<PlayerID>, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM player__players WHERE id = any($1)")
            .bind(ids.into_iter().map(Uuid::from).collect::<Vec<Uuid>>())
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }
    
    pub async fn find_by_faction(fid: FactionID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM player__players WHERE faction_id = $1")
            .bind(i32::from(fid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }
    
    pub async fn find_by_game_and_faction(gid: GameID, fid: FactionID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM player__players WHERE game_id = $1 AND faction_id = $2")
            .bind(Uuid::from(gid))
            .bind(i32::from(fid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }
    
    pub async fn find_by_game(gid: GameID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM player__players WHERE game_id = $1")
            .bind(Uuid::from(gid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }
    
    pub async fn find_by_lobby(lid: LobbyID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM player__players WHERE lobby_id = $1")
            .bind(Uuid::from(lid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn count_by_lobby(lid: LobbyID, db_pool: &PgPool) -> Result<i16> {
        sqlx::query_as("SELECT COUNT(*) FROM player__players WHERE lobby_id = $1")
            .bind(Uuid::from(lid))
            .fetch_one(db_pool).await
            .map(|count: (i64,)| count.0 as i16)
            .map_err(ServerError::from)
    }

    pub async fn check_username_exists(pid: PlayerID, lid: LobbyID, username: String, db_pool: &PgPool) -> Result<bool> {
        sqlx::query_as("SELECT COUNT(*) FROM player__players WHERE lobby_id = $1 AND username = $2 AND id != $3")
            .bind(Uuid::from(lid))
            .bind(username)
            .bind(Uuid::from(pid))
            .fetch_one(db_pool).await
            .map(|count: (i64,)| count.0 > 0)
            .map_err(ServerError::from)
    }

    pub async fn transfer_from_lobby_to_game(lid: &LobbyID, gid: &GameID, db_pool: &PgPool) -> std::result::Result<u64, Error> {
        sqlx::query("UPDATE player__players SET lobby_id = NULL, game_id = $1 WHERE lobby_id = $2")
            .bind(Uuid::from(gid.clone()))
            .bind(Uuid::from(lid.clone()))
            .execute(db_pool).await
    }

    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO player__players (id, wallet, is_ready, is_connected) VALUES($1, $2, $3, $4)")
            .bind(Uuid::from(self.id))
            .bind(self.wallet as i32)
            .bind(self.ready)
            .bind(self.is_connected)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn update<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("UPDATE player__players SET username = $1,
            game_id = $2,
            lobby_id = $3,
            faction_id = $4,
            wallet = $5,
            is_ready = $6,
            is_connected = $7
            WHERE id = $8")
            .bind(self.username.clone())
            .bind(self.game.map(Uuid::from))
            .bind(self.lobby.map(Uuid::from))
            .bind(self.faction.map(i32::from))
            .bind(self.wallet as i32)
            .bind(self.ready)
            .bind(self.is_connected)
            .bind(Uuid::from(self.id))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
}

pub async fn init_player_wallets(players: &mut Vec<Player>, db_pool: &PgPool) -> Result<()> {
    let mut tx = db_pool.begin().await?;
    for player in players.iter_mut() {
        player.wallet = GAME_START_WALLET;
        player.update(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

#[post("/login")]
pub async fn login(state:web::Data<AppState>)
    -> Result<auth::Claims>
{
    let player = Player {
        id: PlayerID(Uuid::new_v4()),
        username: String::from(""),
        lobby: None,
        game: None,
        faction: None,
        ready: false,
        wallet: 0,
        is_connected: true,
    };
    let mut tx = state.db_pool.begin().await?;
    player.insert(&mut tx).await?;
    tx.commit().await?;
    
    Ok(auth::Claims { pid: player.id })
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
        nb_players: (*state.clients()).len()
    }))
}

#[get("/me/")]
pub async fn get_current_player(state:web::Data<AppState>, claims: auth::Claims)
    -> Result<HttpResponse>
{
    Ok(HttpResponse::Ok().json(Player::find(claims.pid, &state.db_pool).await?))
}

#[patch("/me/")]
pub async fn update_current_player(state: web::Data<AppState>, json_data: web::Json<PlayerUpdateData>, claims: auth::Claims)
    -> Result<HttpResponse>
{
    let mut player = Player::find(claims.pid, &state.db_pool).await?;
    let lobby = Lobby::find(player.lobby.unwrap(), &state.db_pool).await?;

    if ! json_data.username.is_empty()
    && json_data.username != player.username
    && Player::check_username_exists(player.id.clone(), lobby.id.clone(), json_data.username.clone(), &state.db_pool).await? {
        return Err(InternalError::PlayerUsernameAlreadyTaken.into());
    }
    player.username = json_data.username.clone();
    player.faction = json_data.faction_id;
    player.ready = json_data.is_ready;
    let mut tx = state.db_pool.begin().await?;
    player.update(&mut tx).await?;
    tx.commit().await?;

    let lobbies = state.lobbies();
    let lobby_server = lobbies.get(&lobby.id).expect("Lobby exists in DB but not in HashMap");
    lobby_server.do_send(protocol::Message::new(
        protocol::Action::PlayerUpdate,
        player.clone(),
        Some(player.id.clone()),
    ));

    if lobby.owner == player.id {
        #[derive(Serialize, Clone)]
        struct LobbyName{
            id: LobbyID,
            name: String
        }
        state.ws_broadcast(&protocol::Message::new(
            protocol::Action::LobbyNameUpdated,
            LobbyName{ id: lobby.id.clone(), name: player.username.clone() },
            Some(player.id),
        ));
    }

    Ok(HttpResponse::NoContent().finish())
}

#[get("/players/")]
pub async fn get_faction_members(state: web::Data<AppState>, info: web::Path<(GameID, FactionID)>)
    -> Result<HttpResponse>
{
    Ok(HttpResponse::Ok().json(Player::find_by_game_and_faction(info.0, info.1, &state.db_pool).await?))
}

#[patch("/players/{player_id}/money/")]
pub async fn transfer_money(state: web::Data<AppState>, info: web::Path<(GameID, FactionID, PlayerID)>, data: web::Json<PlayerMoneyTransferRequest>, claims: auth::Claims)
    -> Result<HttpResponse>
{
    let mut current_player = Player::find(claims.pid, &state.db_pool).await?;
    let mut other_player = Player::find(info.2, &state.db_pool).await?;

    if current_player.faction != other_player.faction {
        return Err(InternalError::Conflict.into());
    }

    if current_player.wallet < data.amount {
        return Err(InternalError::Conflict.into());
    }

    other_player.wallet += data.amount;
    current_player.wallet -= data.amount;

    let mut tx = state.db_pool.begin().await?;
    current_player.update(&mut tx).await?;
    other_player.update(&mut tx).await?;
    tx.commit().await?;

    #[derive(Serialize)]
    pub struct PlayerMoneyTransferData{
        pub amount: usize,
        pub player_id: PlayerID,
    }
    
    let games = state.games();
    let game_server = games.get(&other_player.game.clone().unwrap()).expect("Game exists in DB but not in HashMap");
    game_server.do_send(GameNotifyPlayerMessage(
        other_player.id.clone(),
        protocol::Message::new(
            protocol::Action::PlayerMoneyTransfer,
            PlayerMoneyTransferData{ player_id: current_player.id.clone(), amount: data.amount },
            None,
        )
    ));

    Ok(HttpResponse::NoContent().finish())
}
