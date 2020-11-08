use serde::{Serialize, Deserialize};
use std::collections::HashMap;

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

    #[test]
    fn test_get_attack_formation() {
        let formation = FleetFormation::Center;

        let attack_matrix = formation.get_attack_matrix();

        assert_eq!(attack_matrix.len(), 4);
        assert_eq!(Some(&FleetFormation::Center), attack_matrix.keys().next());
        assert_eq!(Some(&0.85), attack_matrix.values().next());
    }
}