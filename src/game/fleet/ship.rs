use actix_web::{post, web, HttpResponse};
use crate::{
    lib::{
        Result,
        auth::Claims,
        error::{InternalError}
    },
    game::{
        player::{Player},
        game::{GameID, GameDataMessage},
        fleet::fleet::FleetID,
        system::SystemID
    },
    AppState
};

const SHIP_COST: usize = 10;

#[derive(serde::Deserialize)]
pub struct ShipQuantityData {
    pub quantity: usize
}

#[post("/")]
pub async fn add_ship(
    state: web::Data<AppState>,
    info: web::Path<(GameID, SystemID, FleetID)>,
    json_data: web::Json<ShipQuantityData>,
    claims: Claims
) -> Result<HttpResponse> {
    let games = state.games();
    let game = games.get(&info.0).cloned().ok_or(InternalError::GameUnknown)?;
    drop(games);
    
    let locked_data = game.send(GameDataMessage{}).await?;
    let mut data = locked_data.lock().expect("Poisoned lock on game data");
    let system = data.systems.get_mut(&info.1).ok_or(InternalError::SystemUnknown)?;
    let owner_id = system.player.clone();

    let mut player = Player::find(claims.pid, &state.db_pool).await.ok_or(InternalError::PlayerUnknown)?;

    let mut fleet = system.fleets.get_mut(&info.2).ok_or(InternalError::FleetUnknown)?;

    if owner_id != Some(player.id.clone()) || fleet.player != player.id.clone() {
        return Err(InternalError::AccessDenied)?;
    }
    if fleet.destination_system != None {
        return Err(InternalError::FleetAlreadyTravelling)?;
    }
    player.spend(SHIP_COST * json_data.quantity)?;
    fleet.nb_ships += json_data.quantity;

    Ok(HttpResponse::Created().finish())
}
