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
        game::{GameID, GameDataMessage, GamePlayersMessage, GameBroadcastMessage},
        player::PlayerID,
        system::SystemID
    },
    ws::protocol,
    AppState
};

const FLEET_COST: usize = 10;

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct FleetID(Uuid);

#[derive(Serialize, Deserialize, Clone)]
pub struct Fleet{
    pub id: FleetID,
    pub system: SystemID,
    pub player: PlayerID,
    pub nb_ships: usize,
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
    let mut player = players.get_mut(&claims.pid).ok_or(InternalError::PlayerUnknown)?;
    
    if system.player != Some(player.data.id) {
        return Err(InternalError::AccessDenied)?;
    }
    player.spend(FLEET_COST)?;
    let fleet = Fleet{
        id: FleetID(Uuid::new_v4()),
        player: player.data.id.clone(),
        system: system.id.clone(),
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