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
        game::{GameID, GameBuildingConstructionMessage},
        system::system::{System, SystemID},
        player::Player
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

#[derive(Deserialize, Clone)]
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
    pub fn new(sid: SystemID, kind: BuildingKind, data: &BuildingData) -> Building {
        let now = Time::now();

        Building{
            id: BuildingID(Uuid::new_v4()),
            system: sid,
            kind: kind,
            status: BuildingStatus::Constructing,
            created_at: now.clone(),
            built_at: get_construction_time(data, now),
        }
    }

    pub async fn find(bid: BuildingID, db_pool: &PgPool) -> Result<Self> {
        sqlx::query_as("SELECT * FROM map__system_buildings WHERE id = $1")
            .bind(Uuid::from(bid))
            .fetch_one(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_by_system(sid: SystemID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM map__system_buildings WHERE system_id = $1")
            .bind(Uuid::from(sid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_by_kind(kind: BuildingKind, db_pool: &PgPool) -> Result<Vec<Building>> {
        sqlx::query_as("SELECT * FROM map__system_buildings WHERE kind = $1")
            .bind(kind)
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn create(b: Building, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("INSERT INTO map__system_buildings (id, system_id, kind, status, created_at, built_at) VALUES($1, $2, $3, $4, $5, $6)")
            .bind(Uuid::from(b.id))
            .bind(Uuid::from(b.system))
            .bind(b.kind)
            .bind(b.status)
            .bind(b.created_at)
            .bind(b.built_at)
            .execute(tx).await.map_err(ServerError::from)
    }

    pub async fn update(b: Building, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("UPDATE map__system_buildings SET status = $2 WHERE id = $1")
            .bind(Uuid::from(b.id))
            .bind(b.status)
            .execute(tx).await.map_err(ServerError::from)
    }
}

#[get("/")]
pub async fn get_system_buildings(state: web::Data<AppState>, info: web::Path<(GameID, SystemID)>)
    -> Result<HttpResponse>
{
    Ok(HttpResponse::Ok().json(Building::find_by_system(info.1, &state.db_pool).await?))
}

#[post("/")]
pub async fn create_building(
    state: web::Data<AppState>,
    info: web::Path<(GameID,SystemID)>,
    data: web::Json<BuildingRequest>,
    claims: Claims
)
    -> Result<HttpResponse>
{
    let system = System::find(info.1.clone(), &state.db_pool).await?;
    let mut player = Player::find(claims.pid, &state.db_pool).await?;

    if system.player != Some(player.id) {
        return Err(InternalError::AccessDenied)?;
    }

    let buildings = Building::find_by_system(system.id.clone(), &state.db_pool).await?;
    if buildings.len() > 0 {
        return Err(InternalError::Conflict)?;
    }

    let building_data = get_building_data(data.kind);
    player.spend(building_data.cost as usize)?;

    let building = Building::new(info.1.clone(), data.kind, &building_data);

    let mut tx = state.db_pool.begin().await?;
    Player::update(player, &mut tx).await?;
    Building::create(building.clone(), &mut tx).await?;
    tx.commit().await?;

    state.games().get(&info.0).unwrap().do_send(GameBuildingConstructionMessage{ building: building.clone() });

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

fn get_construction_time(data: &BuildingData, from: Time) -> Time {
    let time: DateTime<Utc> = from.into();
    Time(time
        .checked_add_signed(Duration::seconds(data.construction_time as i64))
        .expect("Could not add construction time")
    )
}