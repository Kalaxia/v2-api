use actix_web::{post, web, HttpResponse};
use serde::Deserialize;
use crate::{
    lib::{
        Result,
        error::InternalError,
        auth::Claims
    },
    game::{
        game::{Game, GameID, GameFleetTravelMessage, GameOptionSpeed},
        player::Player,
        fleet::fleet::{Fleet, FleetID, FLEET_RANGE},
        system::system::{System, SystemID, Coordinates},
        fleet::squadron::{FleetSquadron},
    },
    AppState
};
use chrono::{DateTime, Duration, Utc};

#[derive(Deserialize)]
pub struct FleetTravelRequest {
    pub destination_system_id: SystemID,
}

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
        FleetSquadron::find_by_fleet(info.2, &state.db_pool),
        Player::find(claims.pid, &state.db_pool)
    );
    
    let destination_system = ds?;
    let game = g?;
    let system = s?;
    let mut fleet = f?;
    fleet.squadrons = sg?;
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
    check_travel_destination(system.coordinates.clone(), destination_system.coordinates.clone())?;
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

fn check_travel_destination(origin_coords: Coordinates, dest_coords: Coordinates) -> Result<()> {
    let distance = origin_coords.as_distance_to(&dest_coords);

    if distance > FLEET_RANGE.powi(2) {
        return Err(InternalError::FleetInvalidDestination.into());
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use crate::{
        game::{
            game::GameID,
            fleet::{
                squadron::{FleetSquadron, FleetSquadronID, FleetFormation},
            },
            ship::model::ShipModelCategory,
            system::system::{System, SystemID, SystemKind,  Coordinates},
            player::{PlayerID}
        }
    };
    
    #[test]
    fn test_get_travel_time() {
        let time = get_travel_time(
            Coordinates{ x: 1.0, y: 2.0 },
            Coordinates{ x: 4.0, y: 4.0 },
            0.4,
        );
        assert_eq!(10, time.signed_duration_since(Utc::now()).num_seconds());

        let time = get_travel_time(
            Coordinates{ x: 6.0, y: 2.0 },
            Coordinates{ x: 4.0, y: 12.0 },
            0.55,
        );
        assert_eq!(19, time.signed_duration_since(Utc::now()).num_seconds());
    }

    #[test]
    fn test_get_travel_time_coeff() {
        assert_eq!(0.4, get_travel_time_coeff(GameOptionSpeed::Slow));
        assert_eq!(0.55, get_travel_time_coeff(GameOptionSpeed::Medium));
        assert_eq!(0.7, get_travel_time_coeff(GameOptionSpeed::Fast));
    }
}