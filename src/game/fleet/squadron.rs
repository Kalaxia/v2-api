use actix_web::{post , web, HttpResponse};
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use crate::{
    lib::{
        Result,
        error::{ServerError, InternalError},
        auth::Claims
    },
    game::{
        game::game::GameID,
        system::system::{System, SystemID},
        fleet::{
            formation::{FleetFormation},
            fleet::{Fleet, FleetID},
        },
        ship::squadron::{Squadron, SquadronID},
        ship::model::ShipModelCategory
    },
    AppState
};
use sqlx::{PgPool, postgres::{PgRow, PgQueryAs}, FromRow, Executor, Error, Postgres};
use sqlx_core::row::Row;

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct FleetSquadronID(pub Uuid);

impl From<FleetSquadronID> for Uuid {
    fn from(fsid: FleetSquadronID) -> Self { fsid.0 }
}

#[derive(Serialize, Clone, Copy)]
pub struct FleetSquadron {
    pub id: FleetSquadronID,
    pub fleet: FleetID,
    pub formation: FleetFormation,
    pub category: ShipModelCategory,
    pub quantity: u16,
}

#[derive(serde::Deserialize)]
pub struct SquadronAssignmentData {
    pub formation: FleetFormation,
    pub category: ShipModelCategory,
    pub quantity: usize
}

impl<'a> FromRow<'a, PgRow<'a>> for FleetSquadron {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(FleetSquadron {
            id: row.try_get("id").map(FleetSquadronID)?,
            fleet: row.try_get("fleet_id").map(FleetID)?,
            formation: row.try_get("formation")?,
            category: row.try_get("category")?,
            quantity: row.try_get::<i32, _>("quantity")? as u16,
        })
    }
}

impl FleetSquadron {
    pub fn can_fight(&self) -> bool {
        self.quantity > 0
    }

    pub async fn find_by_fleets(ids: Vec<FleetID>, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM fleet__squadrons WHERE fleet_id = any($1)")
            .bind(ids.into_iter().map(Uuid::from).collect::<Vec<Uuid>>())
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_by_fleet(fid: FleetID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM fleet__squadrons WHERE fleet_id = $1")
            .bind(Uuid::from(fid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }
    
    pub async fn find_by_fleet_and_category(fid: FleetID, category: ShipModelCategory, db_pool: &PgPool) -> Result<Option<Self>> {
        sqlx::query_as("SELECT * FROM fleet__squadrons WHERE fleet_id = $1 AND category = $2")
            .bind(Uuid::from(fid))
            .bind(category)
            .fetch_optional(db_pool).await.map_err(ServerError::from)
    }
    
    pub async fn find_by_fleet_and_formation(fid: FleetID, formation: FleetFormation, db_pool: &PgPool) -> Result<Option<Self>> {
        sqlx::query_as("SELECT * FROM fleet__squadrons WHERE fleet_id = $1 AND formation = $2")
            .bind(Uuid::from(fid))
            .bind(formation)
            .fetch_optional(db_pool).await.map_err(ServerError::from)
    }
    
    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
    where
        E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO fleet__squadrons (id, fleet_id, category, formation, quantity) VALUES($1, $2, $3, $4, $5)")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.fleet))
            .bind(self.category)
            .bind(self.formation)
            .bind(self.quantity as i32)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn update<E>(&self, exec: &mut E) -> Result<u64>
    where
        E: Executor<Database = Postgres> {
        sqlx::query("UPDATE fleet__squadrons SET fleet_id = $2, category = $3, formation = $4, quantity = $5 WHERE id = $1")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.fleet))
            .bind(self.category)
            .bind(self.formation)
            .bind(self.quantity as i32)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
    
    pub async fn remove<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("DELETE FROM fleet__squadrons WHERE id = $1")
            .bind(Uuid::from(self.id))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
}

#[post("/")]
pub async fn assign_ships(
    state: web::Data<AppState>,
    info: web::Path<(GameID, SystemID, FleetID)>,
    json_data: web::Json<SquadronAssignmentData>,
    claims: Claims
) -> Result<HttpResponse> {
    let system = System::find(info.1, &state.db_pool).await?;
    let fleet = Fleet::find(&info.2, &state.db_pool).await?;
    let fleet_squadron = FleetSquadron::find_by_fleet_and_formation(
        fleet.id.clone(),
        json_data.formation.clone(),
        &state.db_pool
    ).await?;
    let squadron = Squadron::find_by_system_and_category(
        system.id.clone(),
        json_data.category.clone(),
        &state.db_pool
    ).await?;

    if system.player != Some(claims.pid.clone()) || fleet.player != claims.pid {
        return Err(InternalError::AccessDenied)?;
    }
    let mut available_quantity: u16 = 0;

    if let Some(sg) = squadron.clone() {
        available_quantity += sg.quantity;
    }

    if fleet_squadron.is_some() {
        available_quantity += fleet_squadron.clone().unwrap().quantity;
    }

    if json_data.quantity > available_quantity as usize {
        return Err(InternalError::Conflict)?;
    }

    let mut tx = state.db_pool.begin().await?;
    
    if fleet_squadron.is_none() && json_data.quantity > 0 {
        let fs = FleetSquadron{
            id: FleetSquadronID(Uuid::new_v4()),
            fleet: fleet.id.clone(),
            formation: json_data.formation.clone(),
            quantity: json_data.quantity as u16,
            category: json_data.category.clone(),
        };
        fs.insert(&mut tx).await?;
    } else if fleet_squadron.is_some() && json_data.quantity > 0 {
        let mut fs = fleet_squadron.unwrap();
        if fs.category != json_data.category {
            return Err(InternalError::Conflict)?;
        }
        fs.quantity = json_data.quantity as u16;
        fs.update(&mut tx).await?;
    } else if fleet_squadron.is_some() {
        fleet_squadron.unwrap().remove(&mut tx).await?;
    }

    let remaining_quantity = available_quantity - json_data.quantity as u16;

    if squadron.is_none() && remaining_quantity > 0 {
        let s = Squadron{
            id: SquadronID(Uuid::new_v4()),
            system: system.id.clone(),
            quantity: remaining_quantity,
            category: json_data.category.clone(),
        };
        s.insert(&mut tx).await?;
    } else if squadron.is_some() && remaining_quantity > 0 {
        let mut s = squadron.unwrap();
        s.quantity = remaining_quantity;
        s.update(&mut tx).await?;
    } else if squadron.is_some() {
        squadron.unwrap().remove(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(HttpResponse::NoContent().finish())
}