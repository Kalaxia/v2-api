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
        game::{
            GameID,
            GameDataMessage,
            GamePlayersMessage,
            GameBroadcastMessage,
            GameFleetTravelMessage
        },
        player::PlayerID,
        system::{SystemID, Coordinates}
    },
    ws::protocol,
    AppState
};

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

impl Fleet {
    fn check_travel_destination(&self, origin_coords: &Coordinates, dest_coords: &Coordinates) -> Result<()> {
        if  dest_coords.x > origin_coords.x + 1 ||
            dest_coords.x < origin_coords.x - 1 ||
            dest_coords.y > origin_coords.y + 1 ||
            dest_coords.y < origin_coords.y - 1 {
                return Err(InternalError::FleetInvalidDestination)?;
        }
        Ok(())
    }
}

#[post("/")]
pub async fn create_fleet(state: web::Data<AppState>, info: web::Path<(GameID,SystemID)>, claims: Claims) -> Result<HttpResponse> {
    let mut games = state.games_mut();
    let game = games.get_mut(&info.0).ok_or(InternalError::GameUnknown)?;
    
    let locked_data = game.send(GameDataMessage{}).await?;
    let mut data = locked_data.lock().expect("Poisoned lock on game data");
    let system = data.systems.get_mut(&info.1).ok_or(InternalError::SystemUnknown)?;

    let players_data = game.send(GamePlayersMessage{}).await?;
    let mut players = players_data.lock().expect("Poisoned lock on game players");
    let player = players.get_mut(&claims.pid).ok_or(InternalError::PlayerUnknown)?;
    
    if system.player != Some(player.data.id) {
        return Err(InternalError::AccessDenied)?;
    }
    player.spend(FLEET_COST)?;
    let fleet = Fleet{
        id: FleetID(Uuid::new_v4()),
        player: player.data.id.clone(),
        system: system.id.clone(),
        destination_system: None,
        nb_ships: 1
    };
    system.fleets.insert(fleet.id.clone(), fleet.clone());
    game.do_send(GameBroadcastMessage::<Fleet> {
        message: protocol::Message::<Fleet> {
            action: protocol::Action::FleetCreated,
            data: fleet.clone()
        },
        skip_id: Some(player.data.id.clone())
    });
    Ok(HttpResponse::Created().json(fleet))
}

#[post("/{fleet_id}/travel/")]
pub async fn travel(
    state: web::Data<AppState>,
    info: web::Path<(GameID,SystemID,FleetID)>,
    json_data: web::Json<FleetTravelData>,
    claims: Claims
) -> Result<HttpResponse> {
    let mut games = state.games_mut();
    let game = games.get_mut(&info.0).ok_or(InternalError::GameUnknown)?;
    
    let locked_data = game.send(GameDataMessage{}).await?;
    let mut data = locked_data.lock().expect("Poisoned lock on game data");
    let destination_system = data.systems.get(&json_data.destination_system_id).ok_or(InternalError::SystemUnknown)?.clone();
    let system = data.systems.get_mut(&info.1).ok_or(InternalError::SystemUnknown)?;
    let system_clone = system.clone();
    let mut fleet = system.fleets.get_mut(&info.2).ok_or(InternalError::FleetUnknown)?;

    let players_data = game.send(GamePlayersMessage{}).await?;
    let players = players_data.lock().expect("Poisoned lock on game players");
    let player = players.get(&claims.pid).ok_or(InternalError::PlayerUnknown)?;

    if fleet.player != player.data.id.clone() {
        return Err(InternalError::AccessDenied)?;
    }
    fleet.check_travel_destination(&system_clone.coordinates, &destination_system.coordinates)?;
    fleet.destination_system = Some(destination_system.id.clone());
    game.do_send(GameFleetTravelMessage{ fleet: fleet.clone() });
    Ok(HttpResponse::NoContent().json(fleet))
}