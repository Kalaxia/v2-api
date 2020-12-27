use crate::{
    lib::{
        time::Time,
        error::ServerError,
        Result
    },
    game::{
        fleet::{
            fleet::Fleet,
        },
        game::server::{GameServer, GameConquestMessage},
        player::{Player, PlayerID},
        system::system::{SystemID, System},
    },
    ws::protocol,
};
use chrono::{Duration, Utc};
use serde::Serialize;
use uuid::Uuid;
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Executor, Error, Transaction, Postgres, types::Json};
use sqlx_core::row::Row;

#[derive(Serialize, Clone)]
pub struct Conquest {
    pub player: PlayerID,
    pub system: SystemID,
    pub started_at: Time,
    pub ended_at: Time,
}

impl<'a> FromRow<'a, PgRow<'a>> for Conquest {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(Conquest {
            player: row.try_get("player_id").map(PlayerID)?,
            system: row.try_get("system_id").map(SystemID)?,
            started_at: row.try_get("started_at")?,
            ended_at: row.try_get("ended_at")?,
        })
    }
}

impl Conquest {
    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO fleet__combat__conquests (player_id, system_id, started_at, ended_at) VALUES($1, $2, $3, $4)")
            .bind(Uuid::from(self.player))
            .bind(Uuid::from(self.system))
            .bind(self.started_at)
            .bind(self.ended_at)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn update<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("UPDATE fleet__combat__conquests SET started_at = $2, ended_at = $3 WHERE system_id = $1 AND ended_at IS NULL")
            .bind(Uuid::from(self.system))
            .bind(self.started_at)
            .bind(self.ended_at)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn remove<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("DELETE FROM fleet__combat__conquests WHERE system_id = $1 AND ended_at IS NULL")
            .bind(Uuid::from(self.system))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn find_by_system(sid: &SystemID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM fleet__combat__conquests WHERE system_id = $1")
            .bind(Uuid::from(sid.clone()))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_current_by_system(sid: &SystemID, db_pool: &PgPool) -> Result<Option<Self>> {
        sqlx::query_as("SELECT * FROM fleet__squadrons WHERE system_id = $1 AND ended_at IS NULL")
            .bind(Uuid::from(sid.clone()))
            .fetch_optional(db_pool).await.map_err(ServerError::from)
    }

    pub async fn resume(fleets: Vec<&Fleet>, system: &System, server: &GameServer) -> Result<()> {
        let c = Self::find_current_by_system(&system.id, &server.state.db_pool).await?;
        
        if c.is_none() {
            return Self::new(fleets, system, &server).await;
        }

        let mut conquest = c.unwrap();
        conquest.ended_at = get_remaining_conquest_time(fleets);

        server.state.games().get(&server.id).unwrap().do_send(GameConquestMessage{ conquest });

        Ok(())
    }

    pub async fn new(fleets: Vec<&Fleet>, system: &System, server: &GameServer) -> Result<()> {
        let conquest = Conquest{
            player: fleets[0].player.clone(),
            system: system.id,
            started_at: Time::now(),
            ended_at: get_conquest_time(fleets),
        };
        conquest.insert(&mut &server.state.db_pool).await?;

        server.ws_broadcast(protocol::Message::new(
            protocol::Action::ConquestStarted,
            conquest.clone(),
            None
        ));

        server.state.games().get(&server.id).unwrap().do_send(GameConquestMessage{ conquest });

        Ok(())
    }

    pub async fn end(&self, server: &GameServer) -> Result<()> {
        let mut system = System::find(self.system.clone(), &server.state.db_pool).await?;
        let fleets = Fleet::find_stationed_by_system(&self.system, &server.state.db_pool).await?;

        system.player = Some(self.player.clone());
        system.update(&mut &server.state.db_pool).await?;

        server.ws_broadcast(protocol::Message::new(
            protocol::Action::SystemConquerred,
            system,
            None
        ));

        Ok(())
    }
}

fn get_conquest_time(fleets: Vec<&Fleet>) -> Time {
    (Utc::now() + Duration::seconds(5)).into()
}

fn get_remaining_conquest_time(fleets: Vec<&Fleet>) -> Time {
    (Utc::now() + Duration::seconds(5)).into()
}