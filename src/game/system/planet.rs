use actix_web::{get, post, web, HttpResponse};
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, postgres::{PgRow, PgQueryAs}, FromRow, Executor, Error, Postgres};
use sqlx_core::row::Row;
use futures::executor::block_on;
use crate::{
    task,
    AppState,
    lib::{
        Result,
        auth::Claims,
        log::log,
        error::{ServerError, InternalError},
        time::Time
    },
    game::{
        game::{
            game::{Game, GameID},
            server::{GameServer, GameServerTask},
            option::GameOptionSpeed
        },
        system::system::{System, SystemID},
        player::player::Player
    },
    ws::protocol,
};

#[derive(Serialize, Clone)]
pub struct Planet{
    pub system: SystemID,
    pub biomes: Vec<PlanetBiome>,
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, PartialEq, sqlx::Type)]
#[sqlx(rename = "VARCHAR")]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum PlanetBiome{
    Oceanic,
    Desert,
    Rocky,
    Gas,
    Ice,
    Temperate,
    Tropical,
    Mountain,
    Arid
}

pub enum PlanetTemperature{
    Freezing,
    Cold,
    Normal,
    Hot,
    Melting
}