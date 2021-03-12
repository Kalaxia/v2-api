use actix_web::{get, post, web, HttpResponse};
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, postgres::{PgRow, PgQueryAs}, FromRow, Executor, Error, Postgres};
use sqlx_core::row::Row;
use futures::executor::block_on;
use crate::{
    task,
    AppState,
    lib::{
        Result,
        auth::Claims,
        error::{ServerError, InternalError},
        time::Time
    },
    game::{
        game::{
            game::{Game, GameID},
            server::{GameServer, GameServerTask},
            option::GameOptionSpeed
        },
        system::system::{System, SystemID},
        player::Player
    },
    ws::protocol,
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

#[derive(Deserialize, Serialize, Clone, Copy)]
pub struct BuildingID(pub Uuid);

#[derive(Serialize, Copy, Clone)]
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

impl GameServerTask for Building {
    fn get_task_id(&self) -> String {
        self.id.0.to_string()
    }

    fn get_task_end_time(&self) -> Time {
        self.built_at
    }
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

impl BuildingKind {
    pub const fn to_data(self) -> BuildingData {
        match self {
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
}

impl BuildingData {
    fn into_construction_time(self, from: Time, game_speed: GameOptionSpeed) -> Time {
        let time: DateTime<Utc> = from.into();
        Time(time
            .checked_add_signed(self.into_duration(game_speed))
            .expect("Could not add construction time")
        )
    }

    fn into_duration(self, game_speed: GameOptionSpeed) -> Duration {
        Duration::seconds((self.construction_time as f64 * game_speed.into_coeff()).ceil() as i64)
    }
}

impl Building {
    pub fn new(sid: SystemID, kind: BuildingKind, data: BuildingData, game_speed: GameOptionSpeed) -> Building {
        let now = Time::now();

        Building{
            id: BuildingID(Uuid::new_v4()),
            system: sid,
            kind,
            status: BuildingStatus::Constructing,
            created_at: now.clone(),
            built_at: data.into_construction_time(now, game_speed),
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

    pub async fn count_by_kind_and_system(kind: BuildingKind, sid: SystemID, db_pool: &PgPool) -> Result<u32> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM map__system_buildings WHERE kind = $1 AND system_id = $2 AND status = $3")
            .bind(kind)
            .bind(Uuid::from(sid))
            .bind(BuildingStatus::Operational)
            .fetch_one(db_pool).await.map_err(ServerError::from)?;
        Ok(count.0 as u32)
    }

    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO map__system_buildings (id, system_id, kind, status, created_at, built_at) VALUES($1, $2, $3, $4, $5, $6)")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.system))
            .bind(self.kind)
            .bind(self.status)
            .bind(self.created_at)
            .bind(self.built_at)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn update<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("UPDATE map__system_buildings SET status = $2 WHERE id = $1")
            .bind(Uuid::from(self.id))
            .bind(self.status)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    async fn construct(&mut self, server: &GameServer) -> Result<()> {
        let player = Player::find_system_owner(self.system.clone(), &server.state.db_pool).await?;

        self.status = BuildingStatus::Operational;

        let mut tx = server.state.db_pool.begin().await?;
        self.update(&mut tx).await?;
        tx.commit().await?;

        server.faction_broadcast(player.faction.unwrap(), protocol::Message::new(
            protocol::Action::BuildingConstructed,
            self.clone(),
            None,
        )).await?;

        Ok(())
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
    let game = Game::find(info.0, &state.db_pool).await?;
    let system = System::find(info.1.clone(), &state.db_pool).await?;
    let mut player = Player::find(claims.pid, &state.db_pool).await?;

    if system.player != Some(player.id) {
        return Err(InternalError::AccessDenied.into());
    }

    let buildings = Building::find_by_system(system.id.clone(), &state.db_pool).await?;
    if ! buildings.is_empty() {
        return Err(InternalError::Conflict.into());
    }

    let building_data = data.kind.to_data();
    player.spend(building_data.cost as usize)?;

    let building = Building::new(info.1.clone(), data.kind, building_data, game.game_speed);

    let mut tx = state.db_pool.begin().await?;
    player.update(&mut tx).await?;
    building.insert(&mut tx).await?;
    tx.commit().await?;

    let mut b = building.clone();
    state.games().get(&info.0).unwrap().do_send(task!(building -> move |gs: &GameServer| block_on(b.construct(gs))));

    Ok(HttpResponse::Created().json(building))
}

#[get("/buildings/")]
pub async fn get_buildings_data() -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(vec![
        BuildingKind::Mine.to_data(),
        BuildingKind::Portal.to_data(),
        BuildingKind::Shipyard.to_data(),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_construction_seconds() {
        let shipyard_data = BuildingKind::Shipyard.to_data();

        assert_eq!(24, shipyard_data.into_duration(GameOptionSpeed::Slow).num_seconds());
        assert_eq!(20, shipyard_data.into_duration(GameOptionSpeed::Medium).num_seconds());
        assert_eq!(16, shipyard_data.into_duration(GameOptionSpeed::Fast).num_seconds());
    }
}
