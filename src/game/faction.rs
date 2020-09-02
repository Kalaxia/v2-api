use actix_web::{get, web, HttpResponse};
use serde::{Serialize, Deserialize};
use crate::{
    AppState,
    game::{
        game::GameID,
    },
    lib::{Result, error::*}
};
use uuid::Uuid;
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Error, Transaction};
use sqlx_core::row::Row;

#[derive(Serialize, Deserialize, Clone)]
pub struct Faction{
    pub id: FactionID,
    pub name: String,
    pub color: FactionColor,
}

#[derive(Serialize, Clone)]
pub struct GameFaction{
    pub faction: FactionID,
    pub game: GameID,
    pub victory_points: u16,
}

impl<'a> FromRow<'a, PgRow<'a>> for Faction {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(Faction {
            id: row.try_get::<i32, _>("id").map(|id| FactionID(id as u8))?,
            name: row.try_get("name")?,
            color: row.try_get("color").map(i32::into)?,
        })
    }
}

impl<'a> FromRow<'a, PgRow<'a>> for GameFaction {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(GameFaction{
            faction: row.try_get::<i32, _>("faction_id").map(|id| FactionID(id as u8))?,
            game: row.try_get::<Uuid, _>("game_id").map(GameID)?,
            victory_points: row.try_get::<i16, _>("victory_points").map(|vp| vp as u16)?,
        })
    }
}

#[derive(Serialize, Deserialize, Copy, Clone, Hash, PartialEq, Eq, Debug)]
pub struct FactionID(pub u8);
impl Entity for FactionID {
    const ETYPE : & 'static str = "faction";
}

#[derive(Serialize, Deserialize, Copy, Clone)]
pub struct FactionColor(pub u8, pub u8, pub u8, pub u8);

impl sqlx::Type<sqlx::Postgres> for FactionColor {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        sqlx::postgres::PgTypeInfo::with_name("integer")
    }
}

impl From<i32> for FactionColor {
    fn from(n:i32) -> Self {
        Self(
            ((n >> 24) & 0xff) as u8,
            ((n >> 16) & 0xff) as u8,
            ((n >> 8) & 0xff) as u8,
            ((n >> 0) & 0xff) as u8,
        )
    }
}

impl From<FactionID> for i32 {
    fn from (fid: FactionID) -> i32 { fid.0 as i32 }
}

impl Faction {
    pub async fn find_all(db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM faction__factions ORDER BY id")
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find(fid: FactionID, db_pool: &PgPool) -> Result<Self> {
        sqlx::query_as("SELECT * FROM faction__factions WHERE id = $1")
            .bind(i32::from(fid))
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(&fid))
    }
}

impl GameFaction {
    pub async fn find_all(gid: GameID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM game__factions WHERE game_id = $1 ORDER BY faction_id")
            .bind(Uuid::from(gid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find(gid: GameID, fid: FactionID, db_pool: &PgPool) -> Result<Self> {
        sqlx::query_as("SELECT * FROM game__factions WHERE game_id = $1 AND faction_id = $2")
            .bind(Uuid::from(gid))
            .bind(i32::from(fid))
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(&gid))
    }

    pub async fn create(game_faction: &GameFaction, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("INSERT INTO game__factions(game_id, faction_id, victory_points) VALUES($1, $2, $3)")
            .bind(Uuid::from(game_faction.game))
            .bind(i32::from(game_faction.faction))
            .bind(game_faction.victory_points as i16)
            .execute(tx).await.map_err(ServerError::from)
    }

    pub async fn update(game_faction: &GameFaction, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("UPDATE game__factions SET victory_points = $1 WHERE game_id = $2 AND faction_id = $3")
            .bind(game_faction.victory_points as i16)
            .bind(Uuid::from(game_faction.game))
            .bind(i32::from(game_faction.faction))
            .execute(tx).await.map_err(ServerError::from)
    }
}

#[get("/")]
pub async fn get_factions(state: web::Data<AppState>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(Faction::find_all(&state.db_pool).await?))
}

pub async fn generate_game_factions(gid: GameID, db_pool: &PgPool) -> Result<()> {
    let factions = Faction::find_all(db_pool).await?.into_iter().map(|f| GameFaction{
        faction: f.id,
        game: gid.clone(),
        victory_points: 0,
    });

    let mut tx = db_pool.begin().await?;
    for faction in factions {
        GameFaction::create(&faction, &mut tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
