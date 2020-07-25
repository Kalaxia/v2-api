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
        system::{System, SystemID, Coordinates, get_distance_between},
        fleet::ship::{ShipGroup},
    },
    ws::protocol,
    AppState
};
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Error, Transaction};
use sqlx_core::row::Row;

const FLEET_COST: usize = 10;

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
        self.destination_arrival_date = None;
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
        destination_arrival_date: None,
        ship_groups: vec![],
    };
    let mut tx =state.db_pool.begin().await?;
    Player::update(player.clone(), &mut tx).await?;
    Fleet::create(fleet.clone(), &mut tx).await?;
    tx.commit().await?;

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
    json_data: web::Json<FleetTravelRequest>,
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