use actix_web::{post, web, HttpResponse};
use crate::{
    lib::{
        Result,
        auth::Claims,
        error::{InternalError}
    },
    game::{
        game::{GameID, GameDataMessage, GamePlayersMessage},
        fleet::fleet::FleetID,
        system::SystemID
    },
    AppState
};

const SHIP_COST: usize = 10;

#[post("/")]
pub async fn add_ship(
    state: web::Data<AppState>,
    info: web::Path<(GameID, SystemID, FleetID)>,
    claims: Claims
) -> Result<HttpResponse> {
    let mut games = state.games_mut();
    let game = games.get_mut(&info.0).ok_or(InternalError::GameUnknown)?;
    
    let locked_data = game.send(GameDataMessage{}).await?;
    let mut data = locked_data.lock().expect("Poisoned lock on game data");
    let system = data.systems.get_mut(&info.1).ok_or(InternalError::SystemUnknown)?;
    let owner_id = system.player.clone();

    let players_data = game.send(GamePlayersMessage{}).await?;
    let mut players = players_data.lock().expect("Poisoned lock on game players");
    let player = players.get_mut(&claims.pid).ok_or(InternalError::PlayerUnknown)?;

    let mut fleet = system.fleets.get_mut(&info.2).ok_or(InternalError::FleetUnknown)?;

    if owner_id != Some(player.data.id.clone()) || fleet.player != player.data.id.clone() {
        return Err(InternalError::AccessDenied)?;
    }
    player.spend(SHIP_COST)?;
    fleet.nb_ships += 1;

    Ok(HttpResponse::Created().finish())
}
