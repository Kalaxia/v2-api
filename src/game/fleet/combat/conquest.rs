use actix_web::web;
use crate::{
    task,
    cancel_task,
    lib::{
        time::{ms_to_time, Time},
        log::log,
        error::ServerError,
        Result
    },
    game::{
        faction::FactionID,
        fleet::{
            fleet::{FleetID, Fleet},
        },
        game::{
            game::{Game, GameID},
            option::GameOptionSpeed,
            server::{GameServer, GameServerTask},
        },
        player::{Player, PlayerID},
        system::system::{SystemID, System},
    },
    AppState,
    ws::protocol,
};
use chrono::{DateTime, Utc};
use futures::executor::block_on;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Executor, Error, Transaction, Postgres, types::Json};
use sqlx_core::row::Row;

const CONQUEST_DURATION_MAX: f64 = 60000.0;
const CONQUEST_DURATION_MIN: f64 = 5000.0;
const CONQUEST_STRENGTH_COEFF: f64 = 100.0;

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct ConquestID(pub Uuid);

impl From<ConquestID> for Uuid {
    fn from(cid: ConquestID) -> Self { cid.0 }
}

#[derive(Serialize, Clone)]
pub struct Conquest {
    pub id: ConquestID,
    pub player: PlayerID,
    pub system: SystemID,
    pub fleet: Option<FleetID>,
    pub fleets: Option<Vec<Fleet>>,
    pub is_successful: bool,
    pub is_stopped: bool,
    pub is_over: bool,
    pub percent: f32,
    pub started_at: Time,
    pub ended_at: Time,
}

#[derive(Serialize, Clone)]
pub struct ConquestData {
    pub system: System,
    pub fleets: Vec<Fleet>,
}

impl<'a> FromRow<'a, PgRow<'a>> for Conquest {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(Conquest {
            id: row.try_get("id").map(ConquestID)?,
            player: row.try_get("player_id").map(PlayerID)?,
            system: row.try_get("system_id").map(SystemID)?,
            fleet: None,
            fleets: None,
            is_successful: row.try_get("is_successful")?,
            is_stopped: row.try_get("is_stopped")?,
            is_over: row.try_get("is_over")?,
            percent: row.try_get("percent")?,
            started_at: row.try_get("started_at")?,
            ended_at: row.try_get("ended_at")?,
        })
    }
}

impl GameServerTask for Conquest {
    fn get_task_id(&self) -> String {
        self.id.0.to_string()
    }

    fn get_task_end_time(&self) -> Time {
        self.ended_at
    }
}

impl Conquest {
    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO fleet__combat__conquests (id, player_id, system_id, started_at, ended_at) VALUES($1, $2, $3, $4, $5)")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.player))
            .bind(Uuid::from(self.system))
            .bind(self.started_at)
            .bind(self.ended_at)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn update<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("UPDATE fleet__combat__conquests SET
            started_at = $2,
            ended_at = $3,
            is_successful = $4,
            is_stopped = $5,
            is_over = $6 WHERE id = $1")
            .bind(Uuid::from(self.id))
            .bind(self.started_at)
            .bind(self.ended_at)
            .bind(self.is_successful)
            .bind(self.is_stopped)
            .bind(self.is_over)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn remove<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("DELETE FROM fleet__combat__conquests WHERE id = $1")
            .bind(Uuid::from(self.id))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn find_by_system(sid: &SystemID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM fleet__combat__conquests WHERE system_id = $1")
            .bind(Uuid::from(sid.clone()))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_current_by_system(sid: &SystemID, db_pool: &PgPool) -> Result<Option<Self>> {
        sqlx::query_as("SELECT * FROM fleet__combat__conquests WHERE system_id = $1 AND is_over = false")
            .bind(Uuid::from(sid.clone()))
            .fetch_optional(db_pool).await.map_err(ServerError::from)
    }

    pub async fn remove_fleet(&mut self, system: &System, fleet: &Fleet, server: &GameServer) -> Result<()> {
        let mut fleets = system.retrieve_orbiting_fleets(&server.state.db_pool).await?;
        let game = Game::find(system.game, &server.state.db_pool).await?;
        fleets.retain(|&fid, _| fid != fleet.id);
        // If the current fleet is the only one, the conquest is cancelled
        if fleets.len() < 1 {
            return self.cancel(&server).await;
        }
        server.state.games().get(&server.id).unwrap().do_send(cancel_task!(self));
        self.update_time(fleets.values().collect(), game.game_speed, &server.state.db_pool).await?;

        let mut conquest = self.clone();
        server.state.games().get(&server.id).unwrap().do_send(task!(conquest -> move |server| block_on(conquest.end(&server))));

        Ok(())
    }

    pub async fn update_time(&mut self, fleets: Vec<&Fleet>, game_speed: GameOptionSpeed, mut db_pool: &PgPool) -> Result<()> {
        // If the conquest is currently on and a new fleet joins it, we calculate the progress so the get_conquest_time method can have it
        if !self.is_stopped {
            self.percent = self.calculate_progress();
        }
        self.is_stopped = false;
        self.ended_at = ms_to_time(get_conquest_time(&fleets, self.percent, game_speed));
        self.started_at = Time::now();
        self.update(&mut db_pool).await?;

        Ok(())
    }

    pub async fn cancel(&mut self, server: &GameServer) -> Result<()> {
        self.ended_at = Time::now();
        self.is_over = true;
        self.update(&mut &server.state.db_pool).await?;
        
        let conquest = self.clone();
        server.ws_broadcast(&protocol::Message::new(
            protocol::Action::ConquestCancelled,
            conquest.clone(),
            None
        ));
        server.state.games().get(&server.id).unwrap().do_send(cancel_task!(conquest));

        Ok(())
    }

    pub async fn stop(system: &System, server: &GameServer) -> Result<()> {
        let c = Self::find_current_by_system(&system.id, &server.state.db_pool).await?;
        
        if let Some(mut conquest) = c {
            conquest.halt(&server.state, &server.id).await?;
        }
        Ok(())
    }

    pub async fn halt(&mut self, state: &web::Data<AppState>, game_id: &GameID) -> Result<()> {
        self.is_stopped = true;
        self.percent = self.calculate_progress();
        self.update(&mut &state.db_pool).await?;

        state.games().get(&game_id).unwrap().do_send(cancel_task!(self));

        Ok(())
    }

    pub fn calculate_progress(&self) -> f32 {
        let started_at: DateTime<Utc> = self.started_at.into();
        let ended_at: DateTime<Utc> = self.ended_at.into();

        let total_ms = ended_at.signed_duration_since(started_at).num_milliseconds() as f32;
        let consumed_ms = Utc::now().signed_duration_since(started_at).num_milliseconds() as f32;

        self.percent + (consumed_ms / total_ms)
    }

    pub async fn resume(fleet: &Fleet, system: &System, victor_faction: Option<FactionID>, server: &GameServer) -> Result<()> {
        let c = Self::find_current_by_system(&system.id, &server.state.db_pool).await?;
        let game = Game::find(system.game, &server.state.db_pool).await?;
        let fleets_data = system.retrieve_orbiting_fleets(&server.state.db_pool).await?;
        let fleets = fleets_data.values().collect();
        
        if let Some(mut conquest) = c {
            let conquest_player = Player::find(conquest.player, &server.state.db_pool).await?;
            let games = server.state.games();
            let game_server = games.get(&server.id).unwrap();

            if victor_faction.is_some() && victor_faction != conquest_player.faction {
                conquest.cancel(&server).await?;

                return Self::new(fleet, fleets, system, game.game_speed, &server).await;
            }

            // This case means the fleet is reinforcing a current conquest
            if !conquest.is_stopped {
                game_server.do_send(cancel_task!(conquest));
                game_server.do_send(protocol::Message::new(
                    protocol::Action::FleetArrived,
                    fleet,
                    None,
                ));
            }
            conquest.update_time(fleets, game.game_speed, &server.state.db_pool).await?;

            game_server.do_send(protocol::Message::new(
                protocol::Action::ConquestUpdated,
                conquest.clone(),
                None
            ));
            game_server.do_send(task!(conquest -> move |server| block_on(conquest.end(&server))));

            return Ok(());
        }
        Self::new(fleet, fleets, system, game.game_speed, &server).await
    }

    pub async fn new(fleet: &Fleet, fleets: Vec<&Fleet>, system: &System, game_speed: GameOptionSpeed, server: &GameServer) -> Result<()> {
        let mut conquest = Conquest{
            id: ConquestID(Uuid::new_v4()),
            player: fleets[0].player.clone(),
            system: system.id,
            fleet: Some(fleet.id),
            fleets: Some(fleets.iter().map(|&f| f.clone()).collect()),
            started_at: Time::now(),
            ended_at: ms_to_time(get_conquest_time(&fleets, 0.0, game_speed)),
            percent: 0.0,
            is_stopped: false,
            is_successful: false,
            is_over: false,
        };
        conquest.insert(&mut &server.state.db_pool).await?;

        log(gelf::Level::Informational, "New conquest", "A new conquest has started", vec![
            ("player_id", fleets[0].player.0.to_string()),
            ("fleet_id", fleets[0].id.0.to_string()),
            ("system_id", system.id.0.to_string())
        ], &server.state.logger);

        server.ws_broadcast(&protocol::Message::new(
            protocol::Action::ConquestStarted,
            conquest.clone(),
            None
        ));

        server.state.games().get(&server.id).unwrap().do_send(task!(conquest -> move |server| block_on(conquest.end(&server))));

        Ok(())
    }

    pub async fn end(&mut self, server: &GameServer) -> Result<()> {
        let mut system = System::find(self.system.clone(), &server.state.db_pool).await?;
        let fleets = system.retrieve_orbiting_fleets(&server.state.db_pool).await?.values().cloned().collect();

        self.is_over = true;
        self.update(&mut &server.state.db_pool).await?;

        system.player = Some(self.player.clone());
        system.update(&mut &server.state.db_pool).await?;

        server.ws_broadcast(&protocol::Message::new(
            protocol::Action::SystemConquerred,
            ConquestData{ system, fleets },
            None
        ));

        Ok(())
    }
}

    
fn get_conquest_time(fleets: &Vec<&Fleet>, percent: f32, game_speed: GameOptionSpeed) -> f64 {
    let mut strength = 0;

    for fleet in fleets {
        strength += fleet.get_strength();
    }

    let mut remaining_time = CONQUEST_DURATION_MAX * game_speed.into_conquest_speed();
    if 0.0 < percent {
        remaining_time = remaining_time - (remaining_time * (percent as f64));
    }
    (remaining_time - CONQUEST_STRENGTH_COEFF * strength as f64).max(CONQUEST_DURATION_MIN)
}

#[cfg(test)]
mod tests
{
    use super::*;
    use crate::game::{
        fleet::{
            fleet::Fleet,
            formation::FleetFormation,
            squadron::{FleetSquadron, FleetSquadronID},
        },
        ship::model::ShipModelCategory,
    };
    use uuid::Uuid;

    #[test]
    fn test_get_conquest_time() {
        let mut fleet = get_fleet_mock();
        fleet.squadrons.push(get_squadron_mock(100, ShipModelCategory::Fighter));
        let fleets = vec![&fleet];
        let game_speed = GameOptionSpeed::Medium;

        assert_eq!(50000.0, get_conquest_time(&fleets, 0.0, game_speed));
    }

    #[test]
    fn test_get_conquest_time_in_fast_mode() {
        let mut fleet = get_fleet_mock();
        fleet.squadrons.push(get_squadron_mock(100, ShipModelCategory::Fighter));
        let fleets = vec![&fleet];
        let game_speed = GameOptionSpeed::Fast;

        assert_eq!(38000.0, get_conquest_time(&fleets, 0.0, game_speed));
    }

    #[test]
    fn test_get_conquest_time_with_percent() {
        let mut fleet = get_fleet_mock();
        fleet.squadrons.push(get_squadron_mock(100, ShipModelCategory::Fighter));
        let fleets = vec![&fleet];
        let game_speed = GameOptionSpeed::Medium;

        assert_eq!(20000.0, get_conquest_time(&fleets, 0.5, game_speed));
    }

    #[test]
    fn test_get_conquest_min_time() {
        let mut fleet = get_fleet_mock();
        fleet.squadrons.push(get_squadron_mock(100, ShipModelCategory::Cruiser));
        let fleets = vec![&fleet];
        let game_speed = GameOptionSpeed::Medium;

        assert_eq!(CONQUEST_DURATION_MIN, get_conquest_time(&fleets, 0.0, game_speed));
    }

    #[test]
    fn test_get_conquest_time_with_multiple_fleets() {
        let mut fleet1 = get_fleet_mock();
        fleet1.squadrons.push(get_squadron_mock(50, ShipModelCategory::Fighter));
        fleet1.squadrons.push(get_squadron_mock(50, ShipModelCategory::Fighter));
        let mut fleet2 = get_fleet_mock();
        fleet2.squadrons.push(get_squadron_mock(100, ShipModelCategory::Fighter));
        let fleets = vec![&fleet1, &fleet2];
        let game_speed = GameOptionSpeed::Medium;

        assert_eq!(40000.0, get_conquest_time(&fleets, 0.0, game_speed));
    }

    fn get_fleet_mock() -> Fleet {
        Fleet{
            id: FleetID(Uuid::new_v4()),
            player: PlayerID(Uuid::new_v4()),
            system: SystemID(Uuid::new_v4()),
            destination_system: None,
            destination_arrival_date: None,
            squadrons: vec![],
            is_destroyed: false,
        }
    }

    fn get_squadron_mock(quantity: u16, category: ShipModelCategory) -> FleetSquadron {
        FleetSquadron{
            id: FleetSquadronID(Uuid::new_v4()),
            fleet: FleetID(Uuid::new_v4()),
            formation: FleetFormation::Center,
            quantity,
            category,
        }
    }
}