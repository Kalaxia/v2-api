use actix_web::{get, post, web, HttpResponse};
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Error, Transaction};
use sqlx_core::row::Row;
use crate::{
    AppState,
    lib::{
        Result,
        auth::Claims,
        error::{ServerError, InternalError},
        time::Time
    },
    game::{
        game::GameID,
        system::system::{SystemID}
    }
};

#[derive(Serialize, Clone)]
pub struct Building {
    pub id: BuildingID,
    pub system: SystemID,
    pub kind: BuildingKind,
    pub status: BuildingStatus,
    pub created_at: Time,
    pub built_at: Time,
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, PartialEq, sqlx::Type)]
#[sqlx(rename = "VARCHAR")]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum BuildingStatus {
    Constructing,
    Operational
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, PartialEq, sqlx::Type)]
#[sqlx(rename = "VARCHAR")]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum BuildingKind {
    Mine,
    Portal,
    Shipyard
}

#[derive(Deserialize, Serialize, Clone)]
pub struct BuildingID(pub Uuid);

#[derive(Serialize, Clone)]
pub struct BuildingData {
    pub kind: BuildingKind,
    pub cost: u16,
    pub construction_time: u16,
}

#[derive(Serialize, Clone)]
pub struct BuildingRequest {
    pub kind: BuildingKind,
}

impl From<BuildingID> for Uuid {
    fn from(bid: BuildingID) -> Self { bid.0 }
}

impl<'a> FromRow<'a, PgRow<'a>> for Building {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(Building {
            id: row.try_get("id").map(BuildingID)?,
            system: row.try_get("system_id").map(SystemID)?,
            kind: row.try_get("kind")?,
            status: row.try_get("status")?,
            created_at: row.try_get("created_at")?,
            built_at: row.try_get("built_at")?,
        })
    }
}

impl Building {
    pub async fn create(b: &Building, db_pool: &PgPool) -> Result<u64> {
        sqlx::query("INSERT INTO map__system_buildings (id, system_id, kind, status, created_at, built_at) VALUES($1, $2, $3, $4, $5, $6)")
            .bind(Uuid::from(b.id))
            .bind(Uuid::from(b.system))
            .bind(b.kind)
            .bind(b.status)
            .bind(b.created_at)
            .bind(b.built_at)
            .execute(db_pool).await.map_err(ServerError::from)
    }
}

pub async fn create_building(
    state: web::Data<AppState>,
    info: web::Path<(GameID,SystemID)>,
    data: web::Json<BuildingData>,
    claims: Claims
)
    -> Result<HttpResponse>
{
    let now = Time::now();
    let building = Building{
        id: BuildingID(Uuid::new_v4()),
        system: info.1.clone(),
        kind: data.kind,
        status: BuildingStatus::Constructing,
        created_at: now.clone(),
        built_at: get_construction_time(get_building_data(data.kind), now),
    };

    Building::create(&building, &state.db_pool).await?;


    Ok(HttpResponse::Created().json(building))
}

#[get("/buildings/")]
pub async fn get_buildings_data() -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(vec![
        get_building_data(BuildingKind::Mine),
        get_building_data(BuildingKind::Portal),
        get_building_data(BuildingKind::Shipyard),
    ]))
}

pub fn get_building_data(kind: BuildingKind) -> BuildingData {
    match kind {
        BuildingKind::Mine => BuildingData{
            cost: 250,
            construction_time: 10,
            kind: BuildingKind::Mine,
        },
        BuildingKind::Portal => BuildingData{
            cost: 5000,
            construction_time: 60,
            kind: BuildingKind::Portal,
        },
        BuildingKind::Shipyard => BuildingData {
            cost: 500,
            construction_time: 20,
            kind: BuildingKind::Shipyard,
        }
    }
}

fn get_construction_time(data: BuildingData, from: Time) -> Time {
    Time(from
        .into::<DateTime<Utc>>()
        .checked_add_signed(Duration::seconds(data.construction_time as i64))
        .expect("Could not add construction time")
    )
}