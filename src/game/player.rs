use actix_web::{web, get, patch, post, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Error, Transaction};
use sqlx_core::row::Row;
use crate::{
    AppState,
    game::game::{Game, GAME_START_WALLET, GameNotifyPlayerMessage},
    game::lobby::Lobby,
    game::faction::{FactionID, Faction},
    game::system::system::System,
    lib::{Result, error::{InternalError, ServerError}, auth, uuid::Uuid},
    ws::protocol,
};

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq)]
pub struct Player {
    pub id: Uuid<Player>,
    pub username: String,
    pub game: Option<Uuid<Game>>,
    pub lobby: Option<Uuid<Lobby>>,
    pub faction: Option<FactionID>,
    pub ready: bool,
    pub wallet: usize,
    pub is_connected: bool,
}

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

impl<'a> FromRow<'a, PgRow<'a>> for Player {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(Player {
            id: row.try_get("id").ok().unwrap(),
            username: row.try_get("username")?,
            faction: row.try_get::<i32, _>("faction_id").ok().map(|id| FactionID(id as u8)),
            game: row.try_get("game_id").ok(),
            lobby: row.try_get("lobby_id").ok(),
            wallet: row.try_get::<i32, _>("wallet").map(|w| w as usize)?,
            ready: row.try_get("is_ready")?,
            is_connected: row.try_get("is_connected")?,
        })
    }
}

impl Player {
    pub fn spend(&mut self, amount: usize) -> Result<()> {
        if amount > self.wallet {
            return Err(InternalError::NotEnoughMoney)?;
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
        Player::update(self.clone(), &mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn find(pid: Uuid<Player>, db_pool: &PgPool) -> Result<Self> {
        sqlx::query_as("SELECT * FROM player__players WHERE id = $1")
            .bind(pid)
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(InternalError::PlayerUnknown))
    }

    pub async fn find_system_owner(sid: Uuid<System>, db_pool: &PgPool) -> Result<Self> {
        sqlx::query_as("SELECT p.* FROM map__systems s INNER JOIN player__players p ON p.id = s.player_id WHERE s.id = $1")
            .bind(sid)
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(InternalError::PlayerUnknown))
    }

    pub async fn find_by_ids(ids: Vec<Uuid<Player>>, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM player__players WHERE id = any($1)")
            .bind(ids.into_iter().map(uuid::Uuid::from).collect::<Vec<uuid::Uuid>>())
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }
    
    pub async fn find_by_faction(fid: FactionID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM player__players WHERE faction_id = $1")
            .bind(i32::from(fid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }
    
    pub async fn find_by_game_and_faction(gid: Uuid<Game>, fid: FactionID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM player__players WHERE game_id = $1 AND faction_id = $2")
            .bind(gid)
            .bind(i32::from(fid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }
    
    pub async fn find_by_game(gid: Uuid<Game>, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM player__players WHERE game_id = $1")
            .bind(gid)
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }
    
    pub async fn find_by_lobby(lid: Uuid<Lobby>, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM player__players WHERE lobby_id = $1")
            .bind(lid)
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn count_by_lobby(lid: Uuid<Lobby>, db_pool: &PgPool) -> Result<i16> {
        sqlx::query_as("SELECT COUNT(*) FROM player__players WHERE lobby_id = $1")
            .bind(lid)
            .fetch_one(db_pool).await
            .map(|count: (i64,)| count.0 as i16)
            .map_err(ServerError::from)
    }

    pub async fn check_username_exists(pid: Uuid<Player>, lid: Uuid<Lobby>, username: String, db_pool: &PgPool) -> Result<bool> {
        sqlx::query_as("SELECT COUNT(*) FROM player__players WHERE lobby_id = $1 AND username = $2 AND id != $3")
            .bind(lid)
            .bind(username)
            .bind(pid)
            .fetch_one(db_pool).await
            .map(|count: (i64,)| count.0 > 0)
            .map_err(ServerError::from)
    }

    pub async fn transfer_from_lobby_to_game(lid: &Uuid<Lobby>, gid: &Uuid<Game>, db_pool: &PgPool) -> std::result::Result<u64, Error> {
        sqlx::query("UPDATE player__players SET lobby_id = NULL, game_id = $1 WHERE lobby_id = $2")
            .bind(gid.clone())
            .bind(lid.clone())
            .execute(db_pool).await
    }

    pub async fn create(p: Player, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("INSERT INTO player__players (id, wallet, is_ready, is_connected) VALUES($1, $2, $3, $4)")
            .bind(p.id)
            .bind(p.wallet as i32)
            .bind(p.ready)
            .bind(p.is_connected)
            .execute(tx).await.map_err(ServerError::from)
    }

    pub async fn update(p: Player, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("UPDATE player__players SET username = $1,
            game_id = $2,
            lobby_id = $3,
            faction_id = $4,
            wallet = $5,
            is_ready = $6,
            is_connected = $7
            WHERE id = $8")
            .bind(p.username)
            .bind(p.game)
            .bind(p.lobby)
            .bind(p.faction.map(i32::from))
            .bind(p.wallet as i32)
            .bind(p.ready)
            .bind(p.is_connected)
            .bind(p.id)
            .execute(tx).await.map_err(ServerError::from)
    }
}

pub async fn init_player_wallets(players: &mut Vec<Player>, db_pool: &PgPool) -> Result<()> {
    let mut tx = db_pool.begin().await?;
    for player in players.iter_mut() {
        player.wallet = GAME_START_WALLET;
        Player::update(player.clone(), &mut tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

#[post("/login")]
pub async fn login(state:web::Data<AppState>)
    -> Result<auth::Claims>
{
    let player = Player {
        id: Uuid::new(),
        username: String::from(""),
        lobby: None,
        game: None,
        faction: None,
        ready: false,
        wallet: 0,
        is_connected: true,
    };
    let mut tx =state.db_pool.begin().await?;
    Player::create(player.clone(), &mut tx).await?;
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

    if json_data.username.len() > 0
    && json_data.username != player.username
    && Player::check_username_exists(player.id.clone(), lobby.id.clone(), json_data.username.clone(), &state.db_pool).await? {
        return Err(InternalError::PlayerUsernameAlreadyTaken)?;
    }
    player.username = json_data.username.clone();
    player.faction = json_data.faction_id;
    player.ready = json_data.is_ready;
    let mut tx = state.db_pool.begin().await?;
    Player::update(player.clone(), &mut tx).await?;
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
            id: Uuid<Lobby>,
            name: String
        };
        state.ws_broadcast(protocol::Message::new(
            protocol::Action::LobbyNameUpdated,
            LobbyName{ id: lobby.id.clone(), name: player.username.clone() },
            Some(player.id),
        ));
    }

    Ok(HttpResponse::NoContent().finish())
}

#[get("/players/")]
pub async fn get_faction_members(state: web::Data<AppState>, info: web::Path<(Uuid<Game>, FactionID)>)
    -> Result<HttpResponse>
{
    Ok(HttpResponse::Ok().json(Player::find_by_game_and_faction(info.0, info.1, &state.db_pool).await?))
}

#[patch("/players/{player_id}/money/")]
pub async fn transfer_money(state: web::Data<AppState>, info: web::Path<(Uuid<Game>, Uuid<Faction>, Uuid<Player>)>, data: web::Json<PlayerMoneyTransferRequest>, claims: auth::Claims)
    -> Result<HttpResponse>
{
    let mut current_player = Player::find(claims.pid, &state.db_pool).await?;
    let mut other_player = Player::find(info.2, &state.db_pool).await?;

    if current_player.faction != other_player.faction {
        return Err(InternalError::Conflict)?;
    }

    if current_player.wallet < data.amount {
        return Err(InternalError::Conflict)?;
    }

    other_player.wallet += data.amount;
    current_player.wallet -= data.amount;

    let mut tx = state.db_pool.begin().await?;
    Player::update(current_player.clone(), &mut tx).await?;
    Player::update(other_player.clone(), &mut tx).await?;
    tx.commit().await?;

    #[derive(Serialize)]
    pub struct PlayerMoneyTransferData{
        pub amount: usize,
        pub player_id: Uuid<Player>,
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
