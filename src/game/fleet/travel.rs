use actix_web::{post, patch, web, HttpResponse};
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
        game::{Game, GameID, GameFleetTravelMessage, GameOptionSpeed},
        player::{Player, PlayerID},
        system::system::{System, SystemID, Coordinates},
        fleet::ship::{ShipGroup, ShipGroupID, ShipModelCategory},
    },
    ws::protocol,
    AppState
};
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Error, Transaction};
use sqlx_core::row::Row;


#[post("/travel/")]
pub async fn travel(
    state: web::Data<AppState>,
    info: web::Path<(GameID,SystemID,FleetID,)>,
    json_data: web::Json<FleetTravelRequest>,
    claims: Claims
) -> Result<HttpResponse> {
    let (ds, g, s, f, sg, p) = futures::join!(
        System::find(json_data.destination_system_id, &state.db_pool),
        Game::find(info.0, &state.db_pool),
        System::find(info.1, &state.db_pool),
        Fleet::find(&info.2, &state.db_pool),
        FleetShipGroup::find_by_fleet(info.2, &state.db_pool),
        Player::find(claims.pid, &state.db_pool)
    );
    
    let destination_system = ds?;
    let game = g?;
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
    fleet.destination_arrival_date = Some(get_travel_time(
        system.coordinates,
        destination_system.coordinates,
        get_travel_time_coeff(game.game_speed)
    ).into());
    Fleet::update(fleet.clone(), &state.db_pool).await?;

    let games = state.games();
    let game = games.get(&info.0).cloned().ok_or(InternalError::GameUnknown)?;
    game.do_send(GameFleetTravelMessage{ fleet: fleet.clone() });

    Ok(HttpResponse::Ok().json(fleet))
}

fn get_travel_time(from: Coordinates, to: Coordinates, time_coeff: f64) -> DateTime<Utc> {
    let distance = from.as_distance_to(&to);
    let ms = distance / time_coeff;

    Utc::now().checked_add_signed(Duration::seconds(ms.ceil() as i64)).expect("Could not add travel time")
}

fn get_travel_time_coeff(game_speed: GameOptionSpeed) -> f64 {
    match game_speed {
        GameOptionSpeed::Slow => 0.4,
        GameOptionSpeed::Medium => 0.55,
        GameOptionSpeed::Fast => 0.7,
    }
}