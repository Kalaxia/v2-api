use serde::{Serialize, Deserialize};
use crate::lib::error::InternalError;
use std::str::FromStr;
use std::fmt;

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
    pub fn get_attack_matrix(&self) -> Vec<(Self, f64)> {
        match self {
            FleetFormation::Left => vec![
                (FleetFormation::Right, 1.),
                (FleetFormation::Center, 1.25),
                (FleetFormation::Rear, 1.10),
                (FleetFormation::Left, 1.15),
            ],
            FleetFormation::Center => vec![
                (FleetFormation::Center, 0.85),
                (FleetFormation::Rear, 1.10),
                (FleetFormation::Left, 1.),
                (FleetFormation::Right, 1.),
            ],
            FleetFormation::Right => vec![
                (FleetFormation::Left, 1.),
                (FleetFormation::Center, 1.25),
                (FleetFormation::Rear, 1.10),
                (FleetFormation::Right, 1.15),
            ],
            FleetFormation::Rear => vec![
                (FleetFormation::Left, 1.),
                (FleetFormation::Center, 1.25),
                (FleetFormation::Rear, 1.10),
                (FleetFormation::Right, 1.),
            ]
        }
    }
}

impl FromStr for FleetFormation {
    type Err = InternalError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "center" => Ok(FleetFormation::Center),
            "left" => Ok(FleetFormation::Left),
            "right" => Ok(FleetFormation::Right),
            "rear" => Ok(FleetFormation::Rear),
            _ => Err(InternalError::Conflict)
        }
    }
}

impl fmt::Display for FleetFormation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", format!("{:?}", self).to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_attack_formation() {
        let formation = FleetFormation::Center;

        let attack_matrix = formation.get_attack_matrix();

        assert_eq!(attack_matrix.len(), 4);
        assert_eq!((FleetFormation::Center, 0.85), attack_matrix[0]);
    }
}