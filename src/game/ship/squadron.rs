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
        fleet::squadron::{FleetFormation},
        system::system::{SystemID, System},
        ship::model::ShipModelCategory,
    },
    AppState,
};

#[derive(Serialize, Clone)]
pub struct Squadron {
    pub id: SquadronID,
    pub system: SystemID,
    pub category: ShipModelCategory,
    pub quantity: u16,
}

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct SquadronID(pub Uuid);

impl From<SquadronID> for Uuid {
    fn from(sid: SquadronID) -> Self { sid.0 }
}

impl<'a> FromRow<'a, PgRow<'a>> for Squadron {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(Squadron {
            id: row.try_get("id").map(SquadronID)?,
            system: row.try_get("system_id").map(SystemID)?,
            category: row.try_get("category")?,
            quantity: row.try_get::<i32, _>("quantity")? as u16,
        })
    }
}

impl Squadron {
    pub async fn find_by_system(sid: SystemID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM system__squadrons WHERE system_id = $1")
            .bind(Uuid::from(sid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_by_system_and_category(sid: SystemID, category: ShipModelCategory, db_pool: &PgPool) -> Result<Option<Self>> {
        sqlx::query_as("SELECT * FROM system__squadrons WHERE system_id = $1 AND category = $2")
            .bind(Uuid::from(sid))
            .bind(category)
            .fetch_optional(db_pool).await.map_err(ServerError::from)
    }
    
    pub async fn create<E>(s: Squadron, exec: &mut E) -> Result<u64>
    where
        E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO system__squadrons (id, system_id, category, quantity) VALUES($1, $2, $3, $4)")
            .bind(Uuid::from(s.id))
            .bind(s.system.map(Uuid::from))
            .bind(s.category)
            .bind(s.quantity as i32)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn update<E>(s: &Squadron, exec: &mut E) -> Result<u64>
    where
        E: Executor<Database = Postgres> {
        sqlx::query("UPDATE system__squadrons SET system_id = $2, category = $3, quantity = $4 WHERE id = $1")
            .bind(Uuid::from(s.id))
            .bind(s.system.map(Uuid::from))
            .bind(s.category)
            .bind(s.quantity as i32)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
    
    pub async fn remove(sid: SquadronID, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("DELETE FROM system__squadrons WHERE id = $1")
            .bind(Uuid::from(sid))
            .execute(tx).await.map_err(ServerError::from)
    }
}

#[get("/")]
pub async fn get_system_squadrons(state: web::Data<AppState>, info: web::Path<(GameID, SystemID)>, claims: Claims)
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
    Ok(HttpResponse::Ok().json(Squadron::find_by_system(system.id, &state.db_pool).await?))
}