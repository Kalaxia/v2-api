use actix_web::{post, web, HttpResponse};
use crate::{
    lib::{
        Result,
        auth::Claims,
        error::{InternalError}
    },
    game::{
        player::{Player},
        game::GameID,
        fleet::fleet::{FleetID, Fleet},
        system::{SystemID, System},
    },
    AppState,
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
    let (s, p, f) = futures::join!(
        System::find(info.1, &state.db_pool),
        Player::find(claims.pid, &state.db_pool),
        Fleet::find(&info.2, &state.db_pool)
    );
    let system = s.ok_or(InternalError::SystemUnknown)?;
    let mut player = p?;
    let mut fleet = f.ok_or(InternalError::FleetUnknown)?;

    if system.player.clone() != Some(player.id.clone()) || fleet.player != player.id.clone() {
        return Err(InternalError::AccessDenied)?;
    }
    if fleet.destination_system != None {
        return Err(InternalError::FleetAlreadyTravelling)?;
    }
    player.spend(SHIP_COST * json_data.quantity)?;
    fleet.nb_ships += json_data.quantity;

    Player::update(player, &state.db_pool).await?;
    Fleet::update(fleet, &state.db_pool).await?;

    Ok(HttpResponse::Created().finish())
}
