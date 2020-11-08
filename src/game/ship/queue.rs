use actix_web::{get, post, web, HttpResponse};
use sqlx::{PgPool, Executor, postgres::{PgRow, PgQueryAs}, FromRow, Error, Postgres};
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
        game::{Game, GameID, GameShipQueueMessage},
        ship::model::ShipModelCategory,
        system::system::{SystemID, System},
    },
    AppState,
};

#[derive(Debug, Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct ShipQueueID(pub Uuid);

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

#[derive(serde::Deserialize)]
pub struct ShipQuantityData {
    pub category: ShipModelCategory,
    pub quantity: usize
}

impl From<ShipQueueID> for Uuid {
    fn from(sqid: ShipQueueID) -> Self { sqid.0 }
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

    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO system__ship_queues (id, system_id, category, quantity, created_at, started_at, finished_at) VALUES($1, $2, $3, $4, $5, $6, $7)")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.system))
            .bind(self.category)
            .bind(self.quantity as i32)
            .bind(self.created_at)
            .bind(self.started_at)
            .bind(self.finished_at)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn remove<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("DELETE FROM system__ship_queues WHERE id = $1")
            .bind(Uuid::from(self.id))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
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
    player.update(&mut tx).await?;
    ship_queue.insert(&mut tx).await?;
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