use actix_web::{get, web, HttpResponse};
use sqlx::{PgPool, postgres::{PgRow, PgQueryAs}, FromRow, Executor, Error, Postgres};
use sqlx_core::row::Row;
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use crate::{
    lib::{
        Result,
        auth::Claims,
        error::{ServerError, InternalError},
    },
    game::{
        player::{Player},
        game::game::GameID,
        system::system::{SystemID, System},
        ship::model::ShipModelCategory,
    },
    game::global::AppState,
};

#[derive(Serialize, Clone)]
pub struct Squadron {
    pub id: SquadronID,
    pub system: SystemID,
    pub category: ShipModelCategory,
    pub quantity: u16,
}

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy, Debug)]
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
        sqlx::query_as("SELECT * FROM map__system_squadrons WHERE system_id = $1")
            .bind(Uuid::from(sid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_by_system_and_category(sid: SystemID, category: ShipModelCategory, db_pool: &PgPool) -> Result<Option<Self>> {
        sqlx::query_as("SELECT * FROM map__system_squadrons WHERE system_id = $1 AND category = $2")
            .bind(Uuid::from(sid))
            .bind(category)
            .fetch_optional(db_pool).await.map_err(ServerError::from)
    }
    
    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
    where
        E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO map__system_squadrons (id, system_id, category, quantity) VALUES($1, $2, $3, $4)")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.system))
            .bind(self.category)
            .bind(self.quantity as i32)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn update<E>(&self, exec: &mut E) -> Result<u64>
    where
        E: Executor<Database = Postgres> {
        sqlx::query("UPDATE map__system_squadrons SET system_id = $2, category = $3, quantity = $4 WHERE id = $1")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.system))
            .bind(self.category)
            .bind(self.quantity as i32)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
    
    pub async fn remove<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("DELETE FROM map__system_squadrons WHERE id = $1")
            .bind(Uuid::from(self.id))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn assign<E>(
        squadron: Option<Squadron>,
        system: SystemID,
        category: ShipModelCategory,
        quantity: i32,
        exec: &mut E
    ) -> Result<()>
        where E: Executor<Database = Postgres> {
        if let Some(mut s) = squadron {
            if quantity > 0 {
                s.quantity = quantity as u16;
                s.update(&mut *exec).await?;
            } else {
                s.remove(&mut *exec).await?;
            }
        } else if quantity > 0 {
            let s = Squadron{
                id: SquadronID(Uuid::new_v4()),
                system,
                quantity: quantity as u16,
                category,
            };
            s.insert(&mut *exec).await?;
        }
        Ok(())
    }

    pub async fn assign_existing(system: SystemID, category: ShipModelCategory, mut quantity: i32, mut db_pool: &PgPool) -> Result<()> {
        let squadron = Squadron::find_by_system_and_category(
            system,
            category,
            &db_pool
        ).await?;
        if let Some(sq) = squadron.clone() {
            quantity += sq.quantity as i32;
        }
        Squadron::assign(squadron, system, category, quantity, &mut db_pool).await
    }
}

#[get("/")]
pub async fn get_system_squadrons(state: &AppState, info: web::Path<(GameID, SystemID)>, claims: Claims)
    -> Result<HttpResponse>
{
    let (s, p) = futures::join!(
        System::find(info.1, &state.db_pool),
        Player::find(claims.pid, &state.db_pool),
    );
    let system = s?;
    let player = p?;

    if system.player.clone() != Some(player.id.clone()) {
        return Err(InternalError::AccessDenied.into());
    }
    Ok(HttpResponse::Ok().json(Squadron::find_by_system(system.id, &state.db_pool).await?))
}
