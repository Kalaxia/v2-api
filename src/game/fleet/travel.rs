use actix_web::{post, web, HttpResponse};
use serde::{Serialize, Deserialize};
use crate::{
    lib::{
        Result,
        error::InternalError,
        log::{log, Loggable},
        auth::Claims
    },
    game::{
        game::{
            game::{Game, GameID},
            server::{GameServer, GameFleetTravelMessage},
        },
        player::Player,
        faction::FactionID,
        fleet::{
            combat::{
                battle::Battle,
                conquest::Conquest,
            },
            fleet::{Fleet, FleetID, has_other_fleets_than, FLEET_RANGE},
        },
        system::system::{System, SystemID, Coordinates},
        fleet::squadron::{FleetSquadron},
    },
    ws::protocol,
    AppState
};
use std::collections::HashMap;
use chrono::{Duration, Utc};

#[derive(Deserialize)]
pub struct FleetTravelRequest {
    pub destination_system_id: SystemID,
}

#[derive(Clone)]
pub enum FleetArrivalOutcome {
    Arrived{
        fleet: Fleet,
    },
    Battle{
        defender_faction: Option<FactionID>,
        fleet: Fleet,
        fleets: HashMap<FleetID, Fleet>,
        system: System,
    },
    Colonize{
        system: System,
        fleet: Fleet,
    },
    Conquer{
        system: System,
        fleet: Fleet,
    },
    Defended{
        battle: Battle,
    },
    JoinedBattle{
        fleet: Fleet,
    },
}

#[derive(Serialize, Clone)]
pub struct BattleData {
    pub system: System,
    pub fleet: Fleet,
}

impl From<FleetArrivalOutcome> for Option<protocol::Message> {
    fn from(outcome: FleetArrivalOutcome) -> Self {
        match outcome {
            FleetArrivalOutcome::JoinedBattle { fleet } => Some(protocol::Message::new(
                protocol::Action::FleetJoinedBattle,
                fleet,
                None,
            )),
            FleetArrivalOutcome::Arrived { fleet } => Some(protocol::Message::new(
                protocol::Action::FleetArrived,
                fleet,
                None,
            )),
            _ => None,
        }
    }
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
    let game_id = game.id;
    let system = s?;
    let mut fleet = f?;
    fleet.squadrons = sg?;
    let player = p?;

    if fleet.player != player.id.clone() {
        return Err(InternalError::AccessDenied.into());
    }
    if fleet.destination_system != None {
        return Err(InternalError::FleetAlreadyTravelling.into());
    }
    if !fleet.can_fight() {
        return Err(InternalError::FleetEmpty.into());
    }
    if Battle::count_current_by_system(&system.id, &state.db_pool).await? > 1 {
        return Err(InternalError::Conflict.into());
    }
    check_travel_destination(&system.coordinates, &destination_system.coordinates)?;
    fleet.destination_system = Some(destination_system.id.clone());
    fleet.destination_arrival_date = Some(
        (Utc::now() + get_travel_time(
            &system.coordinates,
            &destination_system.coordinates,
            game.game_speed.into_travel_speed()
        )).into()
    );
    fleet.update(&mut &state.db_pool).await?;
    let games = state.games();
    let game = games.get(&info.0).cloned().ok_or(InternalError::GameUnknown)?;

    if let Some(mut conquest) = Conquest::find_current_by_system(&system.id, &state.db_pool).await? {
        let count = Fleet::count_stationed_by_system(&system.id, &state.db_pool).await?;
        if 1 >= count {
            conquest.halt(&state, &game_id).await?;
        }
    }

    log(
        gelf::Level::Informational,
        "Fleet travel",
        &format!("Fleet {} has gone to system {}", fleet.to_log_message(), destination_system.to_log_message()),
        vec![
            ("fleet_id", fleet.id.to_string()),
            ("system_id", system.id.0.to_string()),
            ("destination_id", destination_system.id.0.to_string()),
        ],
        &state.logger
    );

    game.do_send(GameFleetTravelMessage{ system, fleet: fleet.clone() });

    Ok(HttpResponse::Ok().json(fleet))
}

pub async fn process_fleet_arrival(server: &GameServer, fleet_id: FleetID) -> Result<()> {
    let mut fleet = Fleet::find(&fleet_id, &server.state.db_pool).await?;
    fleet.squadrons = FleetSquadron::find_by_fleet(fleet.id.clone(), &server.state.db_pool).await?;
    let destination_system_id = fleet.destination_system.ok_or(InternalError::SystemUnknown)?;
    let destination_system = System::find(destination_system_id, &server.state.db_pool).await?;
    let player = Player::find(fleet.player, &server.state.db_pool).await?;

    let system_owner = {
        match destination_system.player {
            Some(owner_id) => Some(Player::find(owner_id, &server.state.db_pool).await?),
            None => None,
        }
    };
    fleet.change_system(&destination_system);
    fleet.update(&mut &server.state.db_pool).await?;

    let outcome = resolve_arrival_outcome(&destination_system, &server, fleet.clone(), &player, system_owner).await?;

    if let Some(message) = Option::<protocol::Message>::from(outcome.clone()) {
        server.ws_broadcast(&message).await?;
    }
    
    process_arrival_outcome(&outcome, &server).await
}

async fn resolve_arrival_outcome(system: &System, server: &GameServer, fleet: Fleet, player: &Player, system_owner: Option<Player>) -> Result<FleetArrivalOutcome> {
    // First we check if a battle rages in the destination system. No matter the opponents, the fleet joins in
    if Battle::count_current_by_system(&system.id, &server.state.db_pool).await? > 0 {
        log(
            gelf::Level::Informational,
            "Fleet joined battle",
            &format!("Fleet {} has finished its journey to system {} and encountered an ongoing battle", fleet.to_log_message(), system.to_log_message()),
            vec![
                ("fleet_id", fleet.id.to_string()),
                ("system_id", system.id.0.to_string()),
            ],
            &server.state.logger
        );

        return Ok(FleetArrivalOutcome::JoinedBattle{ fleet });
    }
    match system_owner {
        Some(system_owner) => {
            // Both players have the same faction, the arrived fleet just parks here
            if system_owner.faction == player.faction {
                log(
                    gelf::Level::Informational,
                    "Fleet arrived",
                    &format!("Fleet {} has finished its journey to system {}", fleet.to_log_message(), system.to_log_message()),
                    vec![
                        ("fleet_id", fleet.id.to_string()),
                        ("system_id", system.id.0.to_string()),
                    ],
                    &server.state.logger
                );

                return Ok(FleetArrivalOutcome::Arrived{ fleet });
            }
            // The fleet landed in an enemy system. We check if it is defended by some fleets and initiate a battle
            let fleets = system.retrieve_orbiting_fleets(&server.state.db_pool).await?;
            if has_other_fleets_than(&fleets, &fleet) {
                return Ok(FleetArrivalOutcome::Battle{ system: system.clone(), fleet, fleets, defender_faction: system_owner.faction });
            }
            return Ok(FleetArrivalOutcome::Conquer{ system: system.clone(), fleet });
        },
        None => {
            // The fleet landed in a neutral system. We check if it is currently being colonized by some fleets and initiate a battle
            match Conquest::find_current_by_system(&system.id, &server.state.db_pool).await? {
                Some(current_colonization) => {
                    let colonizer = Player::find(current_colonization.player, &server.state.db_pool).await?;

                    if colonizer.faction != player.faction {
                        let fleets = system.retrieve_orbiting_fleets(&server.state.db_pool).await?;

                        return Ok(FleetArrivalOutcome::Battle{ system: system.clone(), fleet, fleets, defender_faction: None })
                    }
                    // The fleet reinforces the current colonization
                    Ok(FleetArrivalOutcome::Colonize{ system: system.clone(), fleet })
                },
                None => Ok(FleetArrivalOutcome::Colonize{ system: system.clone(), fleet })
            }
        },
    }
}

async fn process_arrival_outcome(outcome: &FleetArrivalOutcome, server: &GameServer) -> Result<()> {
    match outcome {
        FleetArrivalOutcome::Battle { fleet, fleets, system, defender_faction } => Battle::engage(&fleet, &fleets, &system, *defender_faction, &server).await,
        FleetArrivalOutcome::Colonize { fleet, system } => Conquest::resume(fleet, &system, None, &server).await,
        FleetArrivalOutcome::Conquer { fleet, system } => Conquest::resume(fleet, &system, None, &server).await,
        _ => Ok(())
    }
}

#[allow(clippy::suboptimal_flops)]
fn check_travel_destination(origin_coords: &Coordinates, dest_coords: &Coordinates) -> Result<()> {
    let distance = origin_coords.as_distance_to(dest_coords);

    if distance > FLEET_RANGE.powi(2) {
        return Err(InternalError::FleetInvalidDestination.into());
    }

    Ok(())
}

fn get_travel_time(from: &Coordinates, to: &Coordinates, time_coeff: f64) -> Duration {
    let distance = from.as_distance_to(to);
    let ms = distance / time_coeff;

    Duration::seconds(ms.ceil() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        game::{
            system::system::Coordinates
        }
    };
    
    #[test]
    fn test_get_travel_time() {
        let time = get_travel_time(
            &Coordinates{ x: 1.0, y: 2.0 },
            &Coordinates{ x: 4.0, y: 4.0 },
            0.4,
        );
        assert_eq!(10, time.num_seconds());

        let time = get_travel_time(
            &Coordinates{ x: 6.0, y: 2.0 },
            &Coordinates{ x: 4.0, y: 12.0 },
            0.55,
        );
        assert_eq!(19, time.num_seconds());
    }
}
