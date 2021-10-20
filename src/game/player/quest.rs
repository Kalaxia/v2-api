use crate::{
    lib::{
        Result,
        error::ServerError
    },
    game::{
        player::player::{Player, PlayerID},
    }
};
use chrono::{DateTime, Utc};
use futures::executor::block_on;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Executor, Error, Transaction, Postgres, types::Json};
use sqlx_core::row::Row;

pub struct Quest {
    pub id: QuestID,
    pub player: PlayerID,
    pub giver: Option<PlayerID>,
    pub kind: QuestKind,
    pub reward: QuestReward
}

pub struct QuestReward {
    pub kind: QuestRewardKind,
    pub amount: i32
}

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct QuestID(Uuid);

impl From<QuestID> for Uuid {
    fn from(qid: QuestID) -> Self { qid.0 }
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, PartialEq, sqlx::Type)]
#[sqlx(rename = "VARCHAR")]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum QuestKind {
    FleetCreation,
    Conquest,
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, PartialEq, sqlx::Type)]
#[sqlx(rename = "VARCHAR")]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum QuestRewardKind {
    Money,
}

impl Quest {
    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO fleet__combat__conquests (id, player_id, kind) VALUES($1, $2, $3)")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.player))
            .bind(self.kind)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
}

pub async fn give_quest(player: &Player, kind: QuestKind) -> Result<()> {
    Ok(())
}