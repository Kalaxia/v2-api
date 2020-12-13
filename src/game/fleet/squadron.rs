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
            squadron::Squadron,
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
    
    pub async fn find_by_fleet_and_formation(fid: FleetID, formation: &FleetFormation, db_pool: &PgPool) -> Result<Option<Self>> {
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

    pub async fn assign_existing(fid: FleetID, formation: FleetFormation, category: ShipModelCategory, mut quantity: u16, mut db_pool: &PgPool) -> Result<()> {
        let fleet_squadron = FleetSquadron::find_by_fleet_and_formation(
            fid,
            &formation,
            &db_pool
        ).await?;
        if let Some(fs) = fleet_squadron.clone() {
            quantity = quantity + fs.quantity;
        }
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
    let (g, s, f, p, sq, fs) = futures::join!(
        Game::find(info.0, &state.db_pool),
        System::find(info.1, &state.db_pool),
        Fleet::find(&info.2, &state.db_pool),
        Player::find(claims.pid.clone(), &state.db_pool),
        Squadron::find_by_system_and_category(
            info.1,
            &json_data.category,
            &state.db_pool
        ),
        FleetSquadron::find_by_fleet_and_formation(
            info.2,
            &json_data.formation,
            &state.db_pool
        )
    );
    let game = g?;
    let system = s?;
    let fleet = f?;
    let mut player = p?;
    let squadron = sq?;
    let fleet_squadron = fs?;

    if system.player != Some(claims.pid.clone()) || fleet.player != claims.pid {
        return Err(InternalError::AccessDenied)?;
    }

    let available_quantity = get_available_ship_quantity(&squadron, &fleet_squadron);
    let required_quantity = json_data.quantity.clone() as u16;
    let mut assigned_quantity = required_quantity;
    let remaining_quantity: u16;
    let mut ship_queue: Option<ShipQueue> = None;

    if required_quantity > available_quantity {
        assigned_quantity = available_quantity as u16;
        remaining_quantity = 0;
        let assigned_fleet = format!("{}:{}", fleet.id, json_data.formation.to_string());
        let producing_ships = ShipQueue::count_assigned_ships(&assigned_fleet, &json_data.category, &state.db_pool).await?;
        let needed_quantity = get_needed_quantity(required_quantity as i32, available_quantity as i32, producing_ships as i32);

        if needed_quantity > 0 {
            ship_queue = ShipQueue::schedule(
                &mut player,
                system.id,
                json_data.category,
                needed_quantity,
                true,
                Some(assigned_fleet),
                game.game_speed,
                &state.db_pool
            ).await?;
        }
    } else {
        remaining_quantity = available_quantity - required_quantity;
    }

    let mut tx = state.db_pool.begin().await?;
    
    FleetSquadron::assign(
        fleet_squadron,
        fleet.id,
        json_data.formation,
        json_data.category,
        assigned_quantity,
        &mut tx
    ).await?;

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

fn get_needed_quantity(required_quantity: i32, available_quantity: i32, producing_ships: i32) -> u16 {
    let future_quantity = available_quantity + producing_ships;
    if future_quantity <= required_quantity {
        return (required_quantity - future_quantity) as u16;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{
        ship::squadron::SquadronID,
        fleet::{
            fleet::FleetID,
            squadron::{FleetSquadron, FleetSquadronID}
        }
    };

    #[test]
    fn test_get_available_quantity() {
        let squadron = Some(Squadron{
            id: SquadronID(Uuid::new_v4()),
            system: SystemID(Uuid::new_v4()),
            category: ShipModelCategory::Corvette,
            quantity: 5,
        });
        let fleet_squadron = Some(FleetSquadron{
            id: FleetSquadronID(Uuid::new_v4()),
            fleet: FleetID(Uuid::new_v4()),
            formation: FleetFormation::Center,
            category: ShipModelCategory::Corvette,
            quantity: 5,
        });
        let none = None;
        let none_fs = None;

        assert_eq!(10, get_available_ship_quantity(&squadron, &fleet_squadron));
        assert_eq!(5, get_available_ship_quantity(&none, &fleet_squadron));
        assert_eq!(5, get_available_ship_quantity(&squadron, &none_fs));
        assert_eq!(0, get_available_ship_quantity(&none, &none_fs));
    }

    #[test]
    fn test_get_needed_quantity() {
        let data = vec![
            (10, (10, 0, 0)),
            (0, (10, 0, 10)),
            (0, (10, 10, 0)),
            (3, (20, 10, 7)),
            (7, (10, 3, 0)),
            (5, (15, 0, 10)),
        ];

        for (expected, args) in data {
            assert_eq!(expected, get_needed_quantity(args.0, args.1, args.2));
        }
    }
}