use actix_web::{post, web, HttpResponse};
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use crate::{
    lib::{
        Result,
        error::{ServerError, InternalError},
        time::Time,
        auth::Claims
    },
    game::{
        game::{GameID, GameFleetTravelMessage},
        player::{Player, PlayerID},
        system::system::{System, SystemID, Coordinates, get_distance_between},
        fleet::ship::{ShipGroup},
    },
    ws::protocol,
    AppState
};
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Error, Transaction};
use sqlx_core::row::Row;

pub const FLEET_RANGE: f64 = 20.0; // ici j'ai une hypothétique "range", qu'on peut mettre à 1.0 pour l'instant

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct FleetID(pub Uuid);

#[derive(Serialize, Clone)]
pub struct Fleet{
    pub id: FleetID,
    pub system: SystemID,
    pub destination_system: Option<SystemID>,
    pub destination_arrival_date: Option<Time>,
    pub player: PlayerID,
    pub ship_groups: Vec<ShipGroup>,
}

#[derive(Deserialize)]
pub struct FleetTravelRequest {
    pub destination_system_id: SystemID,
}

impl From<FleetID> for Uuid {
    fn from(fid: FleetID) -> Self { fid.0 }
}

impl<'a> FromRow<'a, PgRow<'a>> for Fleet {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(Fleet {
            id: row.try_get("id").map(FleetID)?,
            system: row.try_get("system_id").map(SystemID)?,
            destination_system: row.try_get("destination_id").ok().map(|sid| SystemID(sid)),
            destination_arrival_date: row.try_get("destination_arrival_date")?,
            player: row.try_get("player_id").map(PlayerID)?,
            ship_groups: vec![],
        })
    }
}

impl Fleet {
    fn check_travel_destination(&self, origin_coords: Coordinates, dest_coords: Coordinates) -> Result<()> {
        let distance = get_distance_between(origin_coords, dest_coords);

        if distance > FLEET_RANGE.powi(2) {
            return Err(InternalError::FleetInvalidDestination.into());
        }

        Ok(())
    }

    pub fn change_system(&mut self, system: &System) {
        self.system = system.id.clone();
        self.destination_system = None;
        self.destination_arrival_date = None;
    }

    pub fn can_fight(&self) -> bool {
        !self.ship_groups.is_empty() && self.ship_groups.iter().any(|sg| sg.quantity > 0)
    }

    pub fn is_travelling(&self) -> bool {
        self.destination_system != None
    }

    pub async fn find(fid: &FleetID, db_pool: &PgPool) -> Result<Fleet> {
        sqlx::query_as("SELECT * FROM fleet__fleets WHERE id = $1")
            .bind(Uuid::from(fid.clone()))
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(InternalError::FleetUnknown))
    }

    pub async fn find_stationed_by_system(sid: &SystemID, db_pool: &PgPool) -> Result<Vec<Fleet>> {
        sqlx::query_as("SELECT * FROM fleet__fleets WHERE system_id = $1 AND destination_id IS NULL")
            .bind(Uuid::from(sid.clone()))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn create(f: Fleet, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("INSERT INTO fleet__fleets(id, system_id, player_id) VALUES($1, $2, $3)")
            .bind(Uuid::from(f.id))
            .bind(Uuid::from(f.system))
            .bind(Uuid::from(f.player))
            .execute(tx).await.map_err(ServerError::from)
    }

    pub async fn update(f: Fleet, db_pool: &PgPool) -> Result<u64> {
        sqlx::query("UPDATE fleet__fleets SET system_id=$1, destination_id=$2, destination_arrival_date=$3 WHERE id=$4")
            .bind(Uuid::from(f.system))
            .bind(f.destination_system.map(Uuid::from))
            .bind(f.destination_arrival_date)
            .bind(Uuid::from(f.id))
            .execute(db_pool).await.map_err(ServerError::from)
    }

    pub async fn remove(f: &Fleet, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("DELETE FROM fleet__fleets WHERE id = $1")
            .bind(Uuid::from(f.id))
            .execute(tx).await.map_err(ServerError::from)
    }
}

#[post("/")]
pub async fn create_fleet(state: web::Data<AppState>, info: web::Path<(GameID,SystemID)>, claims: Claims) -> Result<HttpResponse> {
    let system = System::find(info.1, &state.db_pool).await?;
    
    if system.player != Some(claims.pid) {
        return Err(InternalError::AccessDenied)?;
    }
    let fleet = Fleet{
        id: FleetID(Uuid::new_v4()),
        player: claims.pid.clone(),
        system: system.id.clone(),
        destination_system: None,
        destination_arrival_date: None,
        ship_groups: vec![],
    };
    let mut tx = state.db_pool.begin().await?;
    Fleet::create(fleet.clone(), &mut tx).await?;
    tx.commit().await?;

    let games = state.games();
    let game = games.get(&info.0).cloned().ok_or(InternalError::GameUnknown)?;
    game.do_send(protocol::Message::new(
        protocol::Action::FleetCreated,
        fleet.clone(),
        Some(claims.pid.clone()),
    ));
    Ok(HttpResponse::Created().json(fleet))
}

#[post("/travel/")]
pub async fn travel(
    state: web::Data<AppState>,
    info: web::Path<(GameID,SystemID,FleetID,)>,
    json_data: web::Json<FleetTravelRequest>,
    claims: Claims
) -> Result<HttpResponse> {
    let (ds, s, f, sg, p) = futures::join!(
        System::find(json_data.destination_system_id, &state.db_pool),
        System::find(info.1, &state.db_pool),
        Fleet::find(&info.2, &state.db_pool),
        ShipGroup::find_by_fleet(info.2, &state.db_pool),
        Player::find(claims.pid, &state.db_pool)
    );
    
    let destination_system = ds?;
    let system = s?;
    let mut fleet = f?;
    fleet.ship_groups = sg?;
    let player = p?;

    if fleet.player != player.id.clone() {
        return Err(InternalError::AccessDenied)?;
    }
    if fleet.destination_system != None {
        return Err(InternalError::FleetAlreadyTravelling)?;
    }
    if !fleet.can_fight() {
        return Err(InternalError::FleetEmpty)?;
    }
    fleet.check_travel_destination(system.coordinates.clone(), destination_system.coordinates.clone())?;
    fleet.destination_system = Some(destination_system.id.clone());
    fleet.destination_arrival_date = Some(get_travel_time(system.coordinates, destination_system.coordinates).into());
    Fleet::update(fleet.clone(), &state.db_pool).await?;

    let games = state.games();
    let game = games.get(&info.0).cloned().ok_or(InternalError::GameUnknown)?;
    game.do_send(GameFleetTravelMessage{ fleet: fleet.clone() });

    Ok(HttpResponse::Ok().json(fleet))
}

fn get_travel_time(from: Coordinates, to: Coordinates) -> DateTime<Utc> {
    let time_coeff = 0.40;
    let distance = get_distance_between(from, to);
    let ms = distance / time_coeff;

    Utc::now().checked_add_signed(Duration::seconds(ms.ceil() as i64)).expect("Could not add travel time")
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use crate::{
        game::{
            game::GameID,
            fleet::{
                ship::{ShipGroup, ShipGroupID, ShipModelCategory},
            },
            system::system::{System, SystemID, SystemKind,  Coordinates},
            player::{PlayerID}
        }
    };

    #[test]
    fn test_can_fight() {
        let mut fleet = get_fleet_mock();

        assert!(fleet.can_fight());

        fleet.ship_groups[0].quantity = 0;

        assert!(!fleet.can_fight());
        
        fleet.ship_groups = vec![];

        assert!(!fleet.can_fight());
    }

    #[test]
    fn test_is_travelling() {
        let mut fleet = get_fleet_mock();

        assert!(!fleet.is_travelling());

        fleet.destination_system = Some(SystemID(Uuid::new_v4()));

        assert!(fleet.is_travelling());
    }

    #[test]
    fn test_change_system() {
        let mut fleet = get_fleet_mock();
        let system = get_system_mock();

        assert_ne!(fleet.system, system.id.clone());

        fleet.change_system(&system);

        assert_eq!(fleet.system, system.id);
        assert_eq!(fleet.destination_system, None);
        assert_eq!(fleet.destination_arrival_date, None);
    }

    #[test]
    fn test_get_travel_time() {
        let time = get_travel_time(
            Coordinates{ x: 1.0, y: 2.0 },
            Coordinates{ x: 4.0, y: 4.0 }
        );
        assert_eq!(10, time.signed_duration_since(Utc::now()).num_seconds());

        let time = get_travel_time(
            Coordinates{ x: 6.0, y: 2.0 },
            Coordinates{ x: 4.0, y: 12.0 }
        );
        assert_eq!(26, time.signed_duration_since(Utc::now()).num_seconds());
    }

    fn get_fleet_mock() -> Fleet {
        Fleet{
            id: FleetID(Uuid::new_v4()),
            player: PlayerID(Uuid::new_v4()),
            system: SystemID(Uuid::new_v4()),
            destination_system: None,
            destination_arrival_date: None,
            ship_groups: vec![
                ShipGroup{
                    id: ShipGroupID(Uuid::new_v4()),
                    fleet: Some(FleetID(Uuid::new_v4())),
                    system: None,
                    category: ShipModelCategory::Fighter,
                    quantity: 1,
                }
            ]
        }
    }

    fn get_system_mock() -> System {
        System {
            id: SystemID(Uuid::new_v4()),
            game: GameID(Uuid::new_v4()),
            player: None,
            kind: SystemKind::BaseSystem,
            unreachable: false,
            coordinates: Coordinates {
                x: 0.0,
                y: 0.0,
            }
        }
    }
}