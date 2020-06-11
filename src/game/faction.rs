use actix_web::{get, web, HttpResponse};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use crate::{
    AppState,
    lib::Result,
};
use sqlx::{PgPool, postgres::{PgRow, PgQueryAs}, FromRow, Error, QueryAs, Postgres};
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
        Ok(Faction {
            id: FactionID(id as u8),
            name: row.try_get("name")?,
            color: row.try_get("color")?,
        })
    }
}

#[derive(Serialize, Deserialize, Copy, Clone, Hash, PartialEq, Eq, Debug)]
pub struct FactionID(pub u8);
#[derive(Serialize, Deserialize, Copy, Clone, sqlx::Type)]
#[sqlx(rename = "JSONB")]
pub struct FactionColor{
    pub r: i32,
    pub g: i32,
    pub b: i32,
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

    pub async fn find(fid: FactionID, db_pool: &PgPool) -> Option<Self> {
        sqlx::query_as("SELECT * FROM faction__factions WHERE id = ?")
            .bind(i32::from(fid))
            .fetch_one(db_pool).await.ok()
    }
}

#[get("/")]
pub async fn get_factions(state: web::Data<AppState>, db_pool: web::Data<PgPool>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(Faction::find_all(db_pool.get_ref()).await))
}
