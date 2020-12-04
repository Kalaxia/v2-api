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
        game::{
            game::{Game, GameID},
            server::GameShipQueueMessage,
        },
        system::system::{System, SystemID},
        fleet::{
            formation::{FleetFormation},
            fleet::{Fleet, FleetID},
        },
        ship::{
            queue::{ShipQueue},
            squadron::{Squadron, SquadronID},
            model::ShipModelCategory,
        },
        player::Player,
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

#[derive(Serialize, Clone)]
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
    pub quantity: usize,
    pub force_construction: bool
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

    pub async fn assign<E>(
        fleet_squadron: Option<FleetSquadron>,
        fid: FleetID,
        formation: FleetFormation,
        category: ShipModelCategory,
        quantity: u16,
        exec: &mut E
    ) -> Result<()>
        where E: Executor<Database = Postgres> {
        if fleet_squadron.is_none() && quantity > 0 {
            let fs = FleetSquadron{
                id: FleetSquadronID(Uuid::new_v4()),
                fleet: fid.clone(),
                formation: formation.clone(),
                quantity: quantity as u16,
                category: category.clone(),
            };
            fs.insert(&mut *exec).await?;
        } else if fleet_squadron.is_some() && quantity > 0 {
            let mut fs = fleet_squadron.unwrap();
            if fs.category != category {
                return Err(InternalError::Conflict)?;
            }
            fs.quantity = quantity;
            fs.update(&mut *exec).await?;
        } else if fleet_squadron.is_some() {
            fleet_squadron.unwrap().remove(&mut *exec).await?;
        }
        Ok(())
    }

    pub async fn assign_existing(fid: FleetID, formation: FleetFormation, category: ShipModelCategory, quantity: u16, mut db_pool: &PgPool) -> Result<()> {
        let fleet_squadron = FleetSquadron::find_by_fleet_and_formation(
            fid.clone(),
            formation.clone(),
            &db_pool
        ).await?;
        FleetSquadron::assign(fleet_squadron, fid, formation, category, quantity, &mut db_pool).await
    }
}

#[post("/")]
pub async fn assign_ships(
    state: web::Data<AppState>,
    info: web::Path<(GameID, SystemID, FleetID)>,
    json_data: web::Json<SquadronAssignmentData>,
    claims: Claims
) -> Result<HttpResponse> {
    let game = Game::find(info.0, &state.db_pool).await?;
    let system = System::find(info.1, &state.db_pool).await?;
    let fleet = Fleet::find(&info.2, &state.db_pool).await?;
    let mut player = Player::find(claims.pid.clone(), &state.db_pool).await?;
    
    let squadron = Squadron::find_by_system_and_category(
        system.id.clone(),
        json_data.category.clone(),
        &state.db_pool
    ).await?;
    let fleet_squadron = FleetSquadron::find_by_fleet_and_formation(
        fleet.id.clone(),
        json_data.formation.clone(),
        &state.db_pool
    ).await?;

    if system.player != Some(claims.pid.clone()) || fleet.player != claims.pid {
        return Err(InternalError::AccessDenied)?;
    }
    
    let available_quantity = get_available_ship_quantity(&squadron, &fleet_squadron);
    let mut ship_queue: Option<ShipQueue> = None;

    if json_data.quantity > available_quantity as usize {
        if !json_data.force_construction {
            return Err(InternalError::Conflict)?;
        } else {
            let needed_quantity = json_data.quantity as u16 - available_quantity;
            ship_queue = ShipQueue::schedule(
                &mut player,
                system.id,
                json_data.category,
                needed_quantity,
                true,
                Some(format!("{}:{}", fleet.id, json_data.formation.to_string())),
                game.game_speed,
                &state.db_pool
            ).await?;
        }
    }

    let mut tx = state.db_pool.begin().await?;
    
    FleetSquadron::assign(
        fleet_squadron,
        fleet.id,
        json_data.formation,
        json_data.category,
        json_data.quantity as u16,
        &mut tx
    ).await?;

    let remaining_quantity = available_quantity - json_data.quantity as u16;

    Squadron::assign(
        squadron,
        system.id,
        json_data.category,
        remaining_quantity as i32,
        &mut tx
    ).await?;

    tx.commit().await?;

    if let Some(sq) = ship_queue {
        state.games().get(&info.0).unwrap().do_send(GameShipQueueMessage{ ship_queue: sq.clone() });
        return Ok(HttpResponse::Created().json(sq));
    }
    Ok(HttpResponse::NoContent().finish())
}

fn get_available_ship_quantity(squadron: &Option<Squadron>, fleet_squadron: &Option<FleetSquadron>) -> u16 {
    let mut available_quantity: u16 = 0;
    if let Some(sg) = squadron {
        available_quantity += sg.quantity;
    }
    if let Some(fs) = fleet_squadron {
        available_quantity += fs.quantity;
    }
    available_quantity
}
