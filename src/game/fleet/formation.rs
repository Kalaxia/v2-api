use actix_web::{post, patch, web, HttpResponse};
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use crate::{
    lib::{
        Result,
        error::{ServerError, InternalError},
        time::Time,
        auth::Claims
    },
    game::{
        game::GameID,
        player::{Player, PlayerID},
        system::system::{System, SystemID},
        fleet::squadron::{FleetSquadron},
    },
    ws::protocol,
    AppState
};
use std::collections::HashMap;
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Error, Transaction};
use sqlx_core::row::Row;


#[derive(Debug, Serialize, Deserialize, Copy, Clone, Hash, Eq, PartialEq, sqlx::Type)]
#[sqlx(rename = "VARCHAR")]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum FleetFormation {
    Left,
    Center,
    Right,
    Rear,
}

impl FleetFormation {
    pub fn get_attack_matrix(&self) -> HashMap<Self, f64> {
        let mut formation_attack_matrix = HashMap::new();
        match self {
            FleetFormation::Left => {
                formation_attack_matrix.insert(FleetFormation::Right, 1.);
                formation_attack_matrix.insert(FleetFormation::Center, 1.25);
                formation_attack_matrix.insert(FleetFormation::Rear, 1.10);
                formation_attack_matrix.insert(FleetFormation::Left, 1.15);
            },
            FleetFormation::Center => {
                formation_attack_matrix.insert(FleetFormation::Center, 0.85);
                formation_attack_matrix.insert(FleetFormation::Rear, 1.10);
                formation_attack_matrix.insert(FleetFormation::Left, 1.);
                formation_attack_matrix.insert(FleetFormation::Right, 1.);
            },
            FleetFormation::Right => {
                formation_attack_matrix.insert(FleetFormation::Left, 1.);
                formation_attack_matrix.insert(FleetFormation::Center, 1.25);
                formation_attack_matrix.insert(FleetFormation::Rear, 1.10);
                formation_attack_matrix.insert(FleetFormation::Right, 1.15);
            },
            FleetFormation::Rear => {
                formation_attack_matrix.insert(FleetFormation::Left, 1.);
                formation_attack_matrix.insert(FleetFormation::Center, 1.25);
                formation_attack_matrix.insert(FleetFormation::Rear, 1.10);
                formation_attack_matrix.insert(FleetFormation::Right, 1.);
            }
        }
        formation_attack_matrix
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use crate::{
        game::{
            game::GameID,
            fleet::{
                squadron::{FleetSquadron, FleetSquadronID},
            },
            ship::model::ShipModelCategory,
            system::system::{System, SystemID, SystemKind,  Coordinates},
            player::{PlayerID}
        }
    };
}