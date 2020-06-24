use actix_web::{get, web, HttpResponse};
use serde::{Serialize, Deserialize};
use crate::{
    AppState,
    lib::{Result, error::{ServerError, InternalError}},
};
use sqlx::{PgPool, postgres::{PgRow, PgQueryAs}, FromRow, Error};
use sqlx_core::row::Row;

#[derive(Serialize, Deserialize, Clone)]
pub struct Faction{
    pub id: FactionID,
    pub name: String,
    pub color: FactionColor,
}

impl<'a> FromRow<'a, PgRow<'a>> for Faction {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        let id : i32 = row.try_get("id")?;
        let color = row.try_get("color").map(i32::into)?;

        Ok(Faction {
            id: FactionID(id as u8),
            name: row.try_get("name")?,
            color,
        })
    }
}

#[derive(Serialize, Deserialize, Copy, Clone, Hash, PartialEq, Eq, Debug)]
pub struct FactionID(pub u8);
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
    pub async fn find_all(db_pool: &PgPool) -> Vec<Self> {
        let factions: Vec<Self> = sqlx::query_as("SELECT * FROM faction__factions ORDER BY id")
            .fetch_all(db_pool).await.expect("Could not retrieve factions");
        factions
    }

    pub async fn find(fid: FactionID, db_pool: &PgPool) -> Result<Self> {
        sqlx::query_as("SELECT * FROM faction__factions WHERE id = $1")
            .bind(i32::from(fid))
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(InternalError::FactionUnknown))
    }
}

#[get("/")]
pub async fn get_factions(state: web::Data<AppState>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(Faction::find_all(&state.db_pool).await))
}
