use actix_web::{post, patch, web, HttpResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::fmt;
use crate::{
    lib::{
        Result,
        error::{ServerError, InternalError},
        time::Time,
        log::log,
        auth::Claims
    },
    game::{
        game::game::GameID,
        player::{Player, PlayerID},
        system::system::{System, SystemID},
        fleet::squadron::{FleetSquadron},
    },
    ws::protocol,
    AppState
};
use sqlx::{PgPool, postgres::{PgRow, PgQueryAs}, FromRow, Executor, Error, Postgres};
use sqlx_core::row::Row;
use std::collections::HashMap;

pub const FLEET_RANGE: f64 = 20.0;

#[derive(Serialize, Debug, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct FleetID(pub Uuid);

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Fleet{
    pub id: FleetID,
    pub system: SystemID,
    pub destination_system: Option<SystemID>,
    pub destination_arrival_date: Option<Time>,
    pub player: PlayerID,
    pub squadrons: Vec<FleetSquadron>,
    pub is_destroyed: bool,
}

impl fmt::Display for FleetID {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<FleetID> for Uuid {
    fn from(fid: FleetID) -> Self { fid.0 }
}

impl<'a> FromRow<'a, PgRow<'a>> for Fleet {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(Fleet {
            id: row.try_get("id").map(FleetID)?,
            system: row.try_get("system_id").map(SystemID)?,
            destination_system: row.try_get("destination_id").ok().map(SystemID),
            destination_arrival_date: row.try_get("destination_arrival_date")?,
            player: row.try_get("player_id").map(PlayerID)?,
            squadrons: vec![],
            is_destroyed: row.try_get("is_destroyed")?,
        })
    }
}

impl Fleet {
    pub fn change_system(&mut self, system: &System) {
        self.system = system.id.clone();
        self.destination_system = None;
        self.destination_arrival_date = None;
    }

    pub fn can_fight(&self) -> bool {
        !self.squadrons.is_empty() && self.squadrons.iter().any(|s| s.quantity > 0)
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
        sqlx::query_as("SELECT * FROM fleet__fleets WHERE system_id = $1 AND destination_id IS NULL AND is_destroyed = FALSE")
            .bind(Uuid::from(sid.clone()))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn count_stationed_by_system(sid: &SystemID, db_pool: &PgPool) -> Result<i16> {
        sqlx::query_as("SELECT COUNT(*) FROM fleet__fleets WHERE system_id = $1 AND destination_id IS NULL AND is_destroyed = FALSE")
            .bind(Uuid::from(sid.clone()))
            .fetch_one(db_pool).await
            .map(|count: (i64,)| count.0 as i16)
            .map_err(ServerError::from)
    }

    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO fleet__fleets(id, system_id, player_id) VALUES($1, $2, $3)")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.system))
            .bind(Uuid::from(self.player))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn update<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("UPDATE fleet__fleets SET system_id=$1, destination_id=$2, destination_arrival_date=$3, player_id=$4, is_destroyed=$5 WHERE id=$6")
            .bind(Uuid::from(self.system))
            .bind(self.destination_system.map(Uuid::from))
            .bind(self.destination_arrival_date)
            .bind(Uuid::from(self.player))
            .bind(self.is_destroyed)
            .bind(Uuid::from(self.id))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn remove<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("DELETE FROM fleet__fleets WHERE id = $1")
            .bind(Uuid::from(self.id))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub fn get_strength(&self) -> u32 {
        let mut strength = 0;
        for squadron in &self.squadrons {
            strength += squadron.category.to_data().strength as u32 * squadron.quantity as u32; 
        }
        strength
    }
}

#[post("/")]
pub async fn create_fleet(state: web::Data<AppState>, info: web::Path<(GameID,SystemID)>, claims: Claims) -> Result<HttpResponse> {
    let system = System::find(info.1, &state.db_pool).await?;
    
    if system.player != Some(claims.pid) {
        return Err(InternalError::AccessDenied.into());
    }
    let fleet = Fleet{
        id: FleetID(Uuid::new_v4()),
        player: claims.pid.clone(),
        system: system.id.clone(),
        destination_system: None,
        destination_arrival_date: None,
        squadrons: vec![],
        is_destroyed: false,
    };
    let mut tx = state.db_pool.begin().await?;
    fleet.insert(&mut tx).await?;
    tx.commit().await?;

    let games = state.games();
    let game = games.get(&info.0).cloned().ok_or(InternalError::GameUnknown)?;
    game.do_send(protocol::Message::new(
        protocol::Action::FleetCreated,
        fleet.clone(),
        Some(claims.pid.clone()),
    ));
    Ok(HttpResponse::Created().json(fleet))
}

#[patch("/donate/")]
pub async fn donate(
    state: web::Data<AppState>,
    info: web::Path<(GameID,SystemID,FleetID,)>,
    claims: Claims
) -> Result<HttpResponse> {
    let (s, f, sg, p) = futures::join!(
        System::find(info.1, &state.db_pool),
        Fleet::find(&info.2, &state.db_pool),
        FleetSquadron::find_by_fleet(info.2, &state.db_pool),
        Player::find(claims.pid, &state.db_pool)
    );
    let system = s?;
    let mut fleet = f?;
    fleet.squadrons = sg?;
    let player = p?;

    if fleet.player != player.id || system.player.is_none() || fleet.system != system.id {
        return Err(InternalError::Conflict.into());
    }

    let other_player = Player::find(system.player.unwrap(), &state.db_pool).await?;

    if other_player.faction != player.faction || other_player.id == player.id {
        return Err(InternalError::Conflict.into());
    }

    fleet.player = other_player.id;

    fleet.update(&mut &state.db_pool).await?;

    log(gelf::Level::Informational, "Fleet donation", "A fleet has been given", vec![
        ("donator_id", player.id.0.to_string()),
        ("receiver_id", other_player.id.0.to_string()),
        ("fleet_id", fleet.id.0.to_string()),
        ("system_id", system.id.0.to_string()),
    ], &state.logger);

    #[derive(Serialize)]
    pub struct FleetTransferData{
        pub fleet: Fleet,
        pub donator_id: PlayerID,
        pub receiver_id: PlayerID,
    }

    let games = state.games();
    let game_server = games.get(&other_player.game.clone().unwrap()).expect("Game exists in DB but not in HashMap");
    game_server.do_send(protocol::Message::new(
        protocol::Action::FleetTransfer,
        FleetTransferData{ donator_id: player.id, receiver_id: other_player.id, fleet },
        None,
    ));

    Ok(HttpResponse::NoContent().finish())
}

pub fn get_fleet_player_ids(fleets: &HashMap<FleetID, Fleet>) -> Vec<PlayerID> {
    fleets.iter().map(|(_, f)| f.player).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use crate::{
        game::{
            game::game::GameID,
            fleet::{
                formation::{FleetFormation},
                squadron::{FleetSquadron, FleetSquadronID},
            },
            ship::model::ShipModelCategory,
            system::system::{System, SystemID, SystemKind,  Coordinates},
            player::{PlayerID}
        }
    };

    #[test]
    fn test_can_fight() {
        let mut fleet = get_fleet_mock();

        assert!(fleet.can_fight());

        fleet.squadrons[0].quantity = 0;

        assert!(!fleet.can_fight());
        
        fleet.squadrons = vec![];

        assert!(!fleet.can_fight());
    }

    #[test]
    fn test_is_travelling() {
        let mut fleet = get_fleet_mock();

        assert!(!fleet.is_travelling());

        fleet.destination_system = Some(SystemID(Uuid::new_v4()));

        assert!(fleet.is_travelling());
    }

    #[test]
    fn test_change_system() {
        let mut fleet = get_fleet_mock();
        let system = get_system_mock();

        assert_ne!(fleet.system, system.id.clone());

        fleet.change_system(&system);

        assert_eq!(fleet.system, system.id);
        assert_eq!(fleet.destination_system, None);
        assert_eq!(fleet.destination_arrival_date, None);
    }

    fn get_fleet_mock() -> Fleet {
        Fleet{
            id: FleetID(Uuid::new_v4()),
            player: PlayerID(Uuid::new_v4()),
            system: SystemID(Uuid::new_v4()),
            destination_system: None,
            destination_arrival_date: None,
            squadrons: vec![
                FleetSquadron{
                    id: FleetSquadronID(Uuid::new_v4()),
                    fleet: FleetID(Uuid::new_v4()),
                    formation: FleetFormation::Center,
                    category: ShipModelCategory::Fighter,
                    quantity: 1,
                }
            ],
            is_destroyed: false,
        }
    }

    fn get_system_mock() -> System {
        System {
            id: SystemID(Uuid::new_v4()),
            game: GameID(Uuid::new_v4()),
            player: None,
            kind: SystemKind::BaseSystem,
            unreachable: false,
            coordinates: Coordinates {
                x: 0.0,
                y: 0.0,
            }
        }
    }
}
