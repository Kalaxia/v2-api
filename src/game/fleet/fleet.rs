use actix_web::{post, web, HttpResponse};
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use crate::{
    lib::{
        Result,
        error::{ServerError, InternalError},
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
            nb_ships: row.try_get::<i32, _>("nb_ships")? as usize,
        })
    }
}

impl Fleet {
    fn check_travel_destination(&self, origin_coords: Coordinates, dest_coords: Coordinates) -> Result<()> {
        let distance = (dest_coords.x - origin_coords.x).powi(2) + (dest_coords.y - origin_coords.y).powi(2);
        let range = 20f64.powi(2); // ici j'ai une hypothétique "range", qu'on peut mettre à 1.0 pour l'instant

        if distance > range {
            return Err(InternalError::FleetInvalidDestination.into());
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

    pub async fn find(fid: &FleetID, db_pool: &PgPool) -> Result<Fleet> {
        sqlx::query_as("SELECT * FROM fleet__fleets WHERE id = $1")
            .bind(Uuid::from(fid.clone()))
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(InternalError::FleetUnknown))
    }

    pub async fn find_stationed_by_system(sid: &SystemID, db_pool: &PgPool) -> Vec<Fleet> {
        sqlx::query_as("SELECT * FROM fleet__fleets WHERE system_id = $1 AND destination_id IS NULL")
            .bind(Uuid::from(sid.clone()))
            .fetch_all(db_pool).await.expect("Could not retrieve system stationed fleets")
    }

    pub async fn create(f: Fleet, db_pool: &PgPool) -> std::result::Result<u64, Error> {
        sqlx::query("INSERT INTO fleet__fleets(id, system_id, player_id, nb_ships) VALUES($1, $2, $3, $4)")
            .bind(Uuid::from(f.id))
            .bind(Uuid::from(f.system))
            .bind(Uuid::from(f.player))
            .bind(f.nb_ships as i32)
            .execute(db_pool).await
    }

    pub async fn update(f: Fleet, db_pool: &PgPool) -> std::result::Result<u64, Error> {
        sqlx::query("UPDATE fleet__fleets SET system_id=$1, destination_id=$2, nb_ships=$3 WHERE id=$4")
            .bind(Uuid::from(f.system))
            .bind(f.destination_system.map(|sid| Uuid::from(sid)))
            .bind(f.nb_ships as i32)
            .bind(Uuid::from(f.id))
            .execute(db_pool).await
    }

    pub async fn remove_defenders(sid: &SystemID, db_pool: &PgPool) -> std::result::Result<u64, Error> {
        sqlx::query("DELETE FROM fleet__fleets WHERE system_id = $1 AND destination_id IS NULL")
            .bind(Uuid::from(sid.clone()))
            .execute(db_pool).await
    }

    pub async fn remove(f: Fleet, db_pool: &PgPool) -> std::result::Result<u64, Error> {
        sqlx::query("DELETE FROM fleet__fleets WHERE id = $1")
            .bind(Uuid::from(f.id))
            .execute(db_pool).await
    }
}

#[post("/")]
pub async fn create_fleet(state: web::Data<AppState>, info: web::Path<(GameID,SystemID)>, claims: Claims) -> Result<HttpResponse> {
    let system = System::find(info.1, &state.db_pool).await?;
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
    Player::update(player.clone(), &state.db_pool).await?;
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
    
    let destination_system = ds?;
    let system = s?;
    let mut fleet = f?;
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

    Ok(HttpResponse::NoContent().finish())
}
