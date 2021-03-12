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

    /// When attacking, a squadron will have an advantage or disadvantage because of
    /// their relative position with its opponent. This function represent this "position factor"
    /// matrix.
    pub const fn attack_coeff(self, target: Self) -> f64 {
        const COEFFS : [[f64; 4]; 4] =
            [ // Left  Center Right  Rear
                [1.15,  1.25,  1.0, 1.10], // Left
                [ 1.0,  0.85,  1.0, 1.10], // Center
                [ 1.0,  1.25, 1.15, 1.10], // Right
                [ 1.0,  1.25,  1.0, 1.10], // Rear
            ];

        COEFFS[self as usize][target as usize]
    }


    /// When attacking, a squadron will try to find opponents in its attack order. If no opponent
    /// exist in the first attacked position, it searches for opponents in the next position and so
    /// on...
    pub const fn attack_order(self) -> & 'static [Self] {

        // I added every possible formation to each attack order to prevent battles to get stuck
        // (opponents still facing but no one can attack)
        match self {
            FleetFormation::Left => & [FleetFormation::Left, FleetFormation::Center, FleetFormation::Rear, FleetFormation::Right],
            FleetFormation::Center => & [FleetFormation::Center, FleetFormation::Rear, FleetFormation::Left, FleetFormation::Right],
            FleetFormation::Right => & [FleetFormation::Right, FleetFormation::Center, FleetFormation::Rear, FleetFormation::Left],
            FleetFormation::Rear => & [FleetFormation::Left, FleetFormation::Right, FleetFormation::Center, FleetFormation::Rear],
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

        let forms = [
            FleetFormation::Center,
            FleetFormation::Left,
            FleetFormation::Right,
            FleetFormation::Rear,
        ];

        for &form in &forms {
            // a formation needs to have something to attack. if it doesnt then why is it there ?
            assert!(form.attack_order().len() > 0);

            // make sure we won't heal enemies by dealing negative damage
            for &form2 in &forms {
                assert!(form.attack_coeff(form2) > 0.0);
            }
        }
    }
}
