use actix_web::{get, post, web, HttpResponse};
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Executor, Error, Transaction, Postgres};
use sqlx_core::row::Row;
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use crate::{
    lib::{
        Result,
        auth::Claims,
        error::{ServerError, InternalError},
        time::Time,
    },
    game::{
        player::{Player},
        game::{Game, GameID, GameShipQueueMessage, GameOptionSpeed},
        fleet::fleet::{FleetID, Fleet},
        system::system::{SystemID, System},
    },
    AppState,
};

#[derive(Serialize, Clone)]
pub struct ShipGroup {
    pub id: ShipGroupID,
    pub system: Option<SystemID>,
    pub fleet: Option<FleetID>,
    pub category: ShipModelCategory,
    pub quantity: u16,
}

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct ShipGroupID(pub Uuid);

#[derive(Debug, Serialize, Clone)]
pub struct ShipQueue {
    pub id: ShipQueueID,
    pub system: SystemID,
    pub category: ShipModelCategory,
    pub quantity: u16,
    pub created_at: Time,
    pub started_at: Time,
    pub finished_at: Time,
}

#[derive(Debug, Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct ShipQueueID(pub Uuid);

#[derive(Serialize, Copy, Clone)]
pub struct ShipModel {
    pub category: ShipModelCategory,
    pub construction_time: u16,
    pub cost: u16,
    pub damage: u16,
    pub hit_points: u16,
    pub precision: u16,
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, PartialEq, sqlx::Type)]
#[sqlx(rename = "VARCHAR")]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum ShipModelCategory {
    Fighter,
    Corvette,
    Frigate,
    Cruiser,
}

#[derive(Deserialize)]
pub struct ShipQuantityData {
    pub category: ShipModelCategory,
    pub quantity: usize
}

impl From<ShipGroupID> for Uuid {
    fn from(sgid: ShipGroupID) -> Self { sgid.0 }
}

impl From<ShipQueueID> for Uuid {
    fn from(sqid: ShipQueueID) -> Self { sqid.0 }
}

impl<'a> FromRow<'a, PgRow<'a>> for ShipGroup {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(ShipGroup {
            id: row.try_get("id").map(ShipGroupID)?,
            system: row.try_get("system_id").ok().map(SystemID),
            fleet: row.try_get("fleet_id").ok().map(FleetID),
            category: row.try_get("category")?,
            quantity: row.try_get::<i32, _>("quantity")? as u16,
        })
    }
}

impl<'a> FromRow<'a, PgRow<'a>> for ShipQueue {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(ShipQueue {
            id: row.try_get("id").map(ShipQueueID)?,
            system: row.try_get("system_id").map(SystemID)?,
            category: row.try_get("category")?,
            quantity: row.try_get::<i32, _>("quantity")? as u16,
            created_at: row.try_get("created_at")?,
            started_at: row.try_get("started_at")?,
            finished_at: row.try_get("finished_at")?,
        })
    }
}

impl ShipModelCategory {
    pub fn as_data(&self) -> ShipModel {
        match self {
            ShipModelCategory::Fighter => ShipModel{
                category: ShipModelCategory::Fighter,
                construction_time: 400,
                cost: 20,
                damage: 15,
                hit_points: 10,
                precision: 60,
            },
            ShipModelCategory::Corvette => ShipModel{
                category: ShipModelCategory::Corvette,
                construction_time: 1500,
                cost: 140,
                damage: 40,
                hit_points: 60,
                precision: 45,
            },
            ShipModelCategory::Frigate => ShipModel{
                category: ShipModelCategory::Frigate,
                construction_time: 2000,
                cost: 250,
                damage: 25,
                hit_points: 100,
                precision: 50,
            },
            ShipModelCategory::Cruiser => ShipModel{
                category: ShipModelCategory::Cruiser,
                construction_time: 7000,
                cost: 600,
                damage: 80,
                hit_points: 200,
                precision: 45,
            }
        }
    }
}

impl ShipModel {
    pub fn compute_construction_deadline(self, quantity: u16, from: Time, game_speed: GameOptionSpeed) -> Time {
        let datetime: DateTime<Utc> = from.into();

        Time(datetime.checked_add_signed(self.into_duration(quantity, game_speed)).expect("Could not add construction time"))
    }

    pub fn into_duration(self, quantity: u16, game_speed: GameOptionSpeed) -> Duration {
        Duration::milliseconds((
            (quantity as usize * self.construction_time as usize) as f64 * game_speed.into_coeff()
        ).ceil() as i64)
    }
}

impl ShipGroup {
    pub async fn find_by_fleets(ids: Vec<FleetID>, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM fleet__ship_groups WHERE fleet_id = any($1)")
            .bind(ids.into_iter().map(Uuid::from).collect::<Vec<Uuid>>())
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_by_fleet(fid: FleetID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM fleet__ship_groups WHERE fleet_id = $1")
            .bind(Uuid::from(fid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_by_system(sid: SystemID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM fleet__ship_groups WHERE system_id = $1")
            .bind(Uuid::from(sid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_by_fleet_and_category(fid: FleetID, category: ShipModelCategory, db_pool: &PgPool) -> Result<Option<Self>> {
        sqlx::query_as("SELECT * FROM fleet__ship_groups WHERE fleet_id = $1 AND category = $2")
            .bind(Uuid::from(fid))
            .bind(category)
            .fetch_optional(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_by_system_and_category(sid: SystemID, category: ShipModelCategory, db_pool: &PgPool) -> Result<Option<Self>> {
        sqlx::query_as("SELECT * FROM fleet__ship_groups WHERE system_id = $1 AND category = $2")
            .bind(Uuid::from(sid))
            .bind(category)
            .fetch_optional(db_pool).await.map_err(ServerError::from)
    }
    
    pub async fn create<E>(sg: ShipGroup, exec: &mut E) -> Result<u64>
    where
        E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO fleet__ship_groups (id, system_id, fleet_id, category, quantity) VALUES($1, $2, $3, $4, $5)")
            .bind(Uuid::from(sg.id))
            .bind(sg.system.map(Uuid::from))
            .bind(sg.fleet.map(Uuid::from))
            .bind(sg.category)
            .bind(sg.quantity as i32)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn update<E>(sg: &ShipGroup, exec: &mut E) -> Result<u64>
    where
        E: Executor<Database = Postgres> {
        sqlx::query("UPDATE fleet__ship_groups SET system_id = $2, fleet_id = $3, category = $4, quantity = $5 WHERE id = $1")
            .bind(Uuid::from(sg.id))
            .bind(sg.system.map(Uuid::from))
            .bind(sg.fleet.map(Uuid::from))
            .bind(sg.category)
            .bind(sg.quantity as i32)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
    
    pub async fn remove(sgid: ShipGroupID, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("DELETE FROM fleet__ship_groups WHERE id = $1")
            .bind(Uuid::from(sgid))
            .execute(tx).await.map_err(ServerError::from)
    }
}

impl ShipQueue {
    pub async fn find(sqid: ShipQueueID, db_pool: &PgPool) -> Result<Self> {
        sqlx::query_as("SELECT * FROM system__ship_queues WHERE id = $1")
            .bind(Uuid::from(sqid))
            .fetch_one(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_by_system(sid: SystemID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM system__ship_queues WHERE system_id = $1 ORDER BY finished_at DESC")
            .bind(Uuid::from(sid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_last(sid: SystemID, db_pool: &PgPool) -> Result<Self> {
        sqlx::query_as("SELECT * FROM system__ship_queues WHERE system_id = $1 ORDER BY finished_at DESC LIMIT 1")
            .bind(Uuid::from(sid))
            .fetch_one(db_pool).await.map_err(ServerError::from)
    }

    pub async fn create(sq: ShipQueue, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("INSERT INTO system__ship_queues (id, system_id, category, quantity, created_at, started_at, finished_at) VALUES($1, $2, $3, $4, $5, $6, $7)")
            .bind(Uuid::from(sq.id))
            .bind(Uuid::from(sq.system))
            .bind(sq.category)
            .bind(sq.quantity as i32)
            .bind(sq.created_at)
            .bind(sq.started_at)
            .bind(sq.finished_at)
            .execute(tx).await.map_err(ServerError::from)
    }

    pub async fn remove(sqid: ShipQueueID, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("DELETE FROM system__ship_queues WHERE id = $1")
            .bind(Uuid::from(sqid))
            .execute(tx).await.map_err(ServerError::from)
    }
}


#[get("/")]
pub async fn get_system_ship_groups(state: web::Data<AppState>, info: web::Path<(GameID, SystemID)>, claims: Claims)
    -> Result<HttpResponse>
{
    let (s, p) = futures::join!(
        System::find(info.1, &state.db_pool),
        Player::find(claims.pid, &state.db_pool),
    );
    let system = s?;
    let player = p?;

    if system.player.clone() != Some(player.id.clone()) {
        return Err(InternalError::AccessDenied)?;
    }
    Ok(HttpResponse::Ok().json(ShipGroup::find_by_system(system.id, &state.db_pool).await?))
}

#[post("/")]
pub async fn add_ship_queue(
    state: web::Data<AppState>,
    info: web::Path<(GameID, SystemID)>,
    json_data: web::Json<ShipQuantityData>,
    claims: Claims
) -> Result<HttpResponse> {
    let (g, s, p) = futures::join!(
        Game::find(info.0, &state.db_pool),
        System::find(info.1, &state.db_pool),
        Player::find(claims.pid, &state.db_pool),
    );
    let game = g?;
    let system = s?;
    let mut player = p?;
    let ship_model = json_data.category.as_data();

    if system.player.clone() != Some(player.id.clone()) {
        return Err(InternalError::AccessDenied)?;
    }
    player.spend(ship_model.cost as usize * json_data.quantity)?;

    let starts_at = ShipQueue::find_last(system.id.clone(), &state.db_pool).await.ok().map_or(Time::now(), |sq| sq.finished_at);

    let ship_queue = ShipQueue{
        id: ShipQueueID(Uuid::new_v4()),
        system: system.id,
        category: ship_model.category.clone(),
        quantity: json_data.quantity as u16,
        created_at: Time::now(),
        started_at: starts_at.clone(),
        finished_at: ship_model.compute_construction_deadline(json_data.quantity as u16, starts_at, game.game_speed),
    };

    let mut tx = state.db_pool.begin().await?;
    Player::update(player, &mut tx).await?;
    ShipQueue::create(ship_queue.clone(), &mut tx).await?;
    tx.commit().await?;

    state.games().get(&info.0).unwrap().do_send(GameShipQueueMessage{ ship_queue: ship_queue.clone() });

    Ok(HttpResponse::Created().json(ship_queue))
}

#[get("/")]
pub async fn get_ship_queues(state: web::Data<AppState>, info: web::Path<(GameID, SystemID)>, claims: Claims)
    -> Result<HttpResponse>
{
    let (s, p) = futures::join!(
        System::find(info.1, &state.db_pool),
        Player::find(claims.pid, &state.db_pool),
    );
    let system = s?;
    let player = p?;

    if system.player.clone() != Some(player.id.clone()) {
        return Err(InternalError::AccessDenied)?;
    }
    Ok(HttpResponse::Ok().json(ShipQueue::find_by_system(system.id, &state.db_pool).await?))
}

#[post("/")]
pub async fn assign_ships(
    state: web::Data<AppState>,
    info: web::Path<(GameID, SystemID, FleetID)>,
    json_data: web::Json<ShipQuantityData>,
    claims: Claims
) -> Result<HttpResponse> {
    let system = System::find(info.1, &state.db_pool).await?;
    let fleet = Fleet::find(&info.2, &state.db_pool).await?;
    let fleet_ship_group = ShipGroup::find_by_fleet_and_category(
        fleet.id.clone(),
        json_data.category.clone(),
        &state.db_pool
    ).await?;
    let system_ship_group = ShipGroup::find_by_system_and_category(
        system.id.clone(),
        json_data.category.clone(),
        &state.db_pool
    ).await?;

    if system.player != Some(claims.pid.clone()) || fleet.player != claims.pid {
        return Err(InternalError::AccessDenied)?;
    }
    let mut available_quantity: u16 = 0;

    if let Some(sg) = system_ship_group.clone() {
        available_quantity += sg.quantity;
    }

    if fleet_ship_group.is_some() {
        available_quantity += fleet_ship_group.clone().unwrap().quantity;
    }

    if json_data.quantity > available_quantity as usize {
        return Err(InternalError::Conflict)?;
    }

    let mut tx = state.db_pool.begin().await?;
    
    if fleet_ship_group.is_none() && json_data.quantity > 0 {
        ShipGroup::create(ShipGroup{
            id: ShipGroupID(Uuid::new_v4()),
            system: None,
            fleet: Some(fleet.id.clone()),
            quantity: json_data.quantity as u16,
            category: json_data.category.clone(),
        }, &mut tx).await?;
    } else if fleet_ship_group.is_some() && json_data.quantity > 0 {
        let mut sg = fleet_ship_group.unwrap();
        sg.quantity = json_data.quantity as u16;
        ShipGroup::update(&sg, &mut tx).await?;
    } else if fleet_ship_group.is_some() {
        ShipGroup::remove(fleet_ship_group.unwrap().id, &mut tx).await?;
    }

    let remaining_quantity = available_quantity - json_data.quantity as u16;

    if system_ship_group.is_none() && remaining_quantity > 0 {
        ShipGroup::create(ShipGroup{
            id: ShipGroupID(Uuid::new_v4()),
            system: Some(system.id.clone()),
            fleet: None,
            quantity: remaining_quantity,
            category: json_data.category.clone(),
        }, &mut tx).await?;
    } else if system_ship_group.is_some() && remaining_quantity > 0 {
        let mut sg = system_ship_group.unwrap();
        sg.quantity = remaining_quantity;
        ShipGroup::update(&sg, &mut tx).await?;
    } else if system_ship_group.is_some() {
        ShipGroup::remove(system_ship_group.unwrap().id, &mut tx).await?;
    }
    tx.commit().await?;
    Ok(HttpResponse::NoContent().finish())
}

#[get("/ship-models/")]
pub async fn get_ship_models() -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(vec![
        ShipModelCategory::Fighter.as_data(),
        ShipModelCategory::Corvette.as_data(),
        ShipModelCategory::Frigate.as_data(),
        ShipModelCategory::Cruiser.as_data(),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ship_model_data() {
        let fighter = ShipModelCategory::Fighter.as_data();

        assert_eq!(fighter.category, ShipModelCategory::Fighter);

        let cruiser = ShipModelCategory::Cruiser.as_data();

        assert_eq!(cruiser.category, ShipModelCategory::Cruiser);

        assert_ne!(fighter.cost, cruiser.cost);
    }

    #[test]
    fn test_ship_model_construction_milliseconds() {
        let fighter_model = ShipModelCategory::Fighter.as_data();

        assert_eq!(960, fighter_model.into_duration(2, GameOptionSpeed::Slow).num_milliseconds());
        assert_eq!(800, fighter_model.into_duration(2, GameOptionSpeed::Medium).num_milliseconds());
        assert_eq!(640, fighter_model.into_duration(2, GameOptionSpeed::Fast).num_milliseconds());
    }
}