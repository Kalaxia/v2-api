use actix_web::{get, post, web, HttpResponse};
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use crate::{
    lib::{
        Result,
        error::{InternalError},
        auth::Claims
    },
    game::{
        game::{GameID, GameDataMessage},
        player::PlayerID,
        system::SystemID
    },
    AppState
};

const FLEET_COST: usize = 10;

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct FleetID(Uuid);

#[derive(Serialize, Deserialize, Clone)]
pub struct Fleet{
    id: FleetID,
    system: SystemID,
    player: PlayerID,
}
#[derive(Deserialize)]
pub struct FleetCreationData {
    system: SystemID
}

#[post("/")]
pub async fn create_fleet(state: web::Data<AppState>, json_data: web::Json<FleetCreationData>, info: web::Path<(GameID,)>, claims: Claims) -> Result<HttpResponse> {
    let mut games = state.games_mut();
    let game = games.get_mut(&info.0).ok_or(InternalError::GameUnknown)?;
    let mut players = state.players_mut();
    let mut player = players.get_mut(&claims.pid).ok_or(InternalError::PlayerUnknown)?;
    match game.send(GameDataMessage{}).await {
        Ok(locked_data) => {
            let mut data = locked_data.lock().expect("Poisoned lock on game data");
            let game_id = data.id.clone();
            let system = data.systems.get_mut(&json_data.system).ok_or(InternalError::SystemUnknown)?;

            if player.data.game != Some(game_id) || system.player != Some(player.data.id) {
                return Err(InternalError::AccessDenied)?;
            }
            player.spend(FLEET_COST)?;
            let fleet = Fleet{
                id: FleetID(Uuid::new_v4()),
                player: player.data.id.clone(),
                system: system.id.clone(),
            };
            system.fleets.insert(fleet.id.clone(), fleet.clone());
            Ok(HttpResponse::Created().json(fleet))
        },
        _ => Ok(HttpResponse::InternalServerError().finish())
    }
    
}