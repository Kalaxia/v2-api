use serde::{Serialize, Deserialize};

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