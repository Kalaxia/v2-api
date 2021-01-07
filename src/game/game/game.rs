use actix_web::{get, delete, web, HttpResponse};
use actix::prelude::*;
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::{HashMap};
use crate::{
    lib::{
        Result,
        error::{InternalError, ServerError},
        auth::Claims,
    },
    game::{
        fleet::fleet::FLEET_RANGE,
        game::{
            option::{GameOptionSpeed, GameOptionMapSize},
            server::{GameServer, GameRemovePlayerMessage},
        },
        lobby::Lobby,
        player::{PlayerID, Player},
    },
    ws::client::ClientSession,
    AppState,
};
use sqlx::{PgPool, postgres::{PgRow, PgQueryAs}, FromRow, Error, Executor, Postgres};
use sqlx_core::row::Row;

pub const GAME_START_WALLET: usize = 200;
pub const VICTORY_POINTS_PER_MINUTE: i32 = 10;

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct GameID(pub Uuid);

#[derive(Serialize, Deserialize, Clone)]
pub struct Game {
    pub id: GameID,
    pub victory_points: i32,
    pub game_speed: GameOptionSpeed,
    pub map_size: GameOptionMapSize
}

impl From<GameID> for Uuid {
    fn from(gid: GameID) -> Self { gid.0 }
}

impl<'a> FromRow<'a, PgRow<'a>> for Game {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        let id : Uuid = row.try_get("id")?;

        Ok(Game {
            id: GameID(id),
            victory_points: row.try_get::<i32, _>("victory_points")?,
            game_speed: row.try_get("game_speed")?,
            map_size: row.try_get("map_size")?
        })
    }
}

impl Game {
    pub async fn find(gid: GameID, db_pool: &PgPool) -> Result<Self> {
        sqlx::query_as("SELECT * FROM game__games WHERE id = $1")
            .bind(Uuid::from(gid))
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(InternalError::GameUnknown))
    }

    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO game__games(id, game_speed, map_size) VALUES($1, $2, $3)")
            .bind(Uuid::from(self.id))
            .bind(self.game_speed)
            .bind(self.map_size)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn update(game: Game, db_pool: &PgPool) -> Result<u64> {
        sqlx::query("UPDATE game__games SET victory_points = $2 WHERE id = $1")
            .bind(Uuid::from(game.id))
            .bind(game.victory_points)
            .execute(db_pool).await.map_err(ServerError::from)
    }

    pub async fn remove<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("DELETE FROM game__games WHERE id = $1")
            .bind(Uuid::from(self.id))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
}

pub async fn create_game(lobby: &Lobby, state: web::Data<AppState>, clients: HashMap<PlayerID, actix::Addr<ClientSession>>) -> Result<(GameID, Addr<GameServer>)> {
    let id = GameID(Uuid::new_v4());
    
    let game_server = GameServer{
        id: id.clone(),
        state: state.clone(),
        clients: RwLock::new(clients),
    };
    let game = Game{
        id: id.clone(),
        victory_points: 0,
        game_speed: lobby.game_speed.clone(),
        map_size: lobby.map_size.clone(),
    };

    let mut tx = state.db_pool.begin().await?;
    game.insert(&mut tx).await?;
    tx.commit().await?;

    Player::transfer_from_lobby_to_game(&lobby.id, &id, &state.db_pool).await?;

    Ok((id, game_server.start()))
}

#[get("/{id}/players/")]
pub async fn get_players(state: web::Data<AppState>, info: web::Path<(GameID,)>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(Player::find_by_game(info.0, &state.db_pool).await?))
}

#[delete("/{id}/players/")]
pub async fn leave_game(state:web::Data<AppState>, claims: Claims, info: web::Path<(GameID,)>)
    -> Result<HttpResponse>
{
    let game = Game::find(info.0, &state.db_pool).await?;
    let mut player = Player::find(claims.pid, &state.db_pool).await?;

    if player.game != Some(game.id) {
        Err(InternalError::NotInLobby)?
    }
    player.reset(&state.db_pool).await?;

    let games = state.games().await;
    let game_server = games.get(&game.id).expect("Game exists in DB but not in HashMap");
    let (client, is_empty) = Arc::try_unwrap(game_server.send(GameRemovePlayerMessage(player.id.clone())).await?).ok().unwrap();
    state.add_client(&player.id, client.clone()).await;
    if is_empty {
        drop(games);
        state.clear_game(&game).await?;
    }
    Ok(HttpResponse::NoContent().finish())
}

#[get("/constants/")]
pub async fn get_game_constants() -> Result<HttpResponse> {
    #[derive(Serialize, Clone)]
    pub struct GameConstants {
        fleet_range: f64,
        victory_points_per_minute: i32,
    }
    Ok(HttpResponse::Ok().json(GameConstants{
        fleet_range: FLEET_RANGE,
        victory_points_per_minute: VICTORY_POINTS_PER_MINUTE,
    }))
}
