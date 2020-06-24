use actix_web::{post, web, HttpResponse};
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use crate::{
    lib::{
        Result,
        error::{InternalError},
        auth::Claims
    },
    game::{
        game::{GameID, GameFleetTravelMessage},
        player::{Player, PlayerID},
        system::{System, SystemID, Coordinates}
    },
    ws::protocol,
    AppState
};
use sqlx::{PgPool, postgres::{PgRow, PgQueryAs}, FromRow, Error};
use sqlx_core::row::Row;

pub const FLEET_TRAVEL_TIME: u8 = 10;
const FLEET_COST: usize = 10;

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct FleetID(Uuid);

#[derive(Serialize, Deserialize, Clone)]
pub struct Fleet{
    pub id: FleetID,
    pub system: SystemID,
    pub destination_system: Option<SystemID>,
    pub player: PlayerID,
    pub nb_ships: usize,
}

#[derive(Deserialize)]
pub struct FleetTravelData {
    pub destination_system_id: SystemID,
}

impl From<FleetID> for Uuid {
    fn from(fid: FleetID) -> Self { fid.0 }
}

impl<'a> FromRow<'a, PgRow<'a>> for Fleet {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        let id : Uuid = row.try_get("id")?;
        let sid : Uuid = row.try_get("system_id")?;
        let pid : Uuid = row.try_get("player_id")?;
        let destination_id = match row.try_get("destination_id") {
            Ok(sid) => Some(SystemID(sid)),
            Err(_) => None
        };

        Ok(Fleet {
            id: FleetID(id),
            system: SystemID(sid),
            destination_system: destination_id,
            player: PlayerID(pid),
            nb_ships: row.try_get::<i64, _>("nb_ships")? as usize,
        })
    }
}

impl Fleet {
    fn check_travel_destination(&self, origin_coords: Coordinates, dest_coords: Coordinates) -> Result<()> {
        if  dest_coords.x > origin_coords.x + 1 ||
            (dest_coords.x as i8) < (origin_coords.x as i8) - 1 ||
            dest_coords.y > origin_coords.y + 1 ||
            (dest_coords.y as i8) < (origin_coords.y as i8) - 1 {
                return Err(InternalError::FleetInvalidDestination)?;
        }
        Ok(())
    }

    pub fn change_system(&mut self, system: &mut System) {
        self.system = system.id.clone();
        self.destination_system = None;
    }

    pub fn is_travelling(&self) -> bool {
        self.destination_system != None
    }

    pub async fn find(fid: &FleetID, db_pool: &PgPool) -> Option<Fleet> {
        sqlx::query_as("SELECT * FROM fleet__fleets WHERE id = $id")
            .bind(Uuid::from(fid.clone()))
            .fetch_one(db_pool).await.ok()
    }

    pub async fn find_stationed_by_system(sid: &SystemID, db_pool: &PgPool) -> Vec<Fleet> {
        sqlx::query_as("SELECT * FROM fleet__fleets WHERE system_id = $1 AND destination_id IS NULL")
            .bind(Uuid::from(sid.clone()))
            .fetch_all(db_pool).await.expect("Could not retrieve system stationed fleets")
    }

    pub async fn create(f: Fleet, db_pool: &PgPool) -> std::result::Result<u64, Error> {
        sqlx::query("INSERT INTO fleet__fleets(id, system_id, player_id, nb_ships) VALUES($1, $32, $3, $4)")
            .bind(Uuid::from(f.id))
            .bind(Uuid::from(f.system))
            .bind(Uuid::from(f.player))
            .bind(f.nb_ships as i64)
            .execute(db_pool).await
    }

    pub async fn update(f: Fleet, db_pool: &PgPool) -> std::result::Result<u64, Error> {
        sqlx::query("UPDATE fleet__fleets SET system_id = $1, destination_id = $2, nb_ships = $3 WHERE id = $4")
            .bind(Uuid::from(f.system))
            .bind(f.destination_system.map(|sid| Uuid::from(sid)))
            .bind(f.nb_ships as i64)
            .bind(Uuid::from(f.id))
            .execute(db_pool).await
    }

    pub async fn remove_defenders(sid: &SystemID, db_pool: &PgPool) -> std::result::Result<u64, Error> {
        sqlx::query("REMOVE FROM fleet__fleets WHERE system_id = $1 AND WHERE destination_id IS NULL")
            .bind(Uuid::from(sid.clone()))
            .execute(db_pool).await
    }
}

#[post("/")]
pub async fn create_fleet(state: web::Data<AppState>, info: web::Path<(GameID,SystemID)>, claims: Claims) -> Result<HttpResponse> {
    let system = System::find(info.1, &state.db_pool).await.ok_or(InternalError::SystemUnknown)?;
    let mut player = Player::find(claims.pid, &state.db_pool).await?;
    
    if system.player != Some(player.id) {
        return Err(InternalError::AccessDenied)?;
    }
    player.spend(FLEET_COST)?;
    let fleet = Fleet{
        id: FleetID(Uuid::new_v4()),
        player: player.id.clone(),
        system: system.id.clone(),
        destination_system: None,
        nb_ships: 1
    };
    Fleet::create(fleet.clone(), &state.db_pool).await?;

    let games = state.games();
    let game = games.get(&info.0).cloned().ok_or(InternalError::GameUnknown)?;
    game.do_send(protocol::Message::new(
        protocol::Action::FleetCreated,
        fleet.clone(),
        Some(player.id.clone()),
    ));
    Ok(HttpResponse::Created().json(fleet))
}

#[post("/travel/")]
pub async fn travel(
    state: web::Data<AppState>,
    info: web::Path<(GameID,SystemID,FleetID,)>,
    json_data: web::Json<FleetTravelData>,
    claims: Claims
) -> Result<HttpResponse> {
    let (ds, s, f, p) = futures::join!(
        System::find(json_data.destination_system_id, &state.db_pool),
        System::find(info.1, &state.db_pool),
        Fleet::find(&info.2, &state.db_pool),
        Player::find(claims.pid, &state.db_pool)
    );
    
    let destination_system = ds.ok_or(InternalError::SystemUnknown)?;
    let system = s.ok_or(InternalError::SystemUnknown)?;
    let mut fleet = f.ok_or(InternalError::FleetUnknown)?;
    let player = p?;

    if fleet.player != player.id.clone() {
        return Err(InternalError::AccessDenied)?;
    }
    if fleet.destination_system != None {
        return Err(InternalError::FleetAlreadyTravelling)?;
    }
    fleet.check_travel_destination(system.coordinates, destination_system.coordinates)?;
    fleet.destination_system = Some(destination_system.id.clone());
    Fleet::update(fleet.clone(), &state.db_pool).await?;

    let games = state.games();
    let game = games.get(&info.0).cloned().ok_or(InternalError::GameUnknown)?;
    game.do_send(GameFleetTravelMessage{ fleet: fleet.clone() });

    Ok(HttpResponse::NoContent().json(fleet))
}
