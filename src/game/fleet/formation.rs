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

    /// When attacking, a squadron will have an advantage or disadvantage because of
    /// their relative position with its opponent. This function represent this "position factor"
    /// matrix.
    pub fn attack_coeff(self, target: Self) -> f64 {
        const COEFFS : [[f64; 4]; 4] =
            [ // Left  Center Right  Rear  
                [1.15,  1.25,  1.0, 1.10], // Left
                [ 1.0,  0.85,  1.0, 1.10], // Center
                [ 1.0,  1.25, 1.15, 1.10], // Right
                [ 1.0,  1.25,  1.0, 1.10], // Rear
            ];

        COEFFS[self as usize][self as usize]
    }


    /// When attacking, a squadron will try to find opponents in its attack order. If no opponent
    /// exist in the first attacked position, it searches for opponents in the next position and so
    /// on...
    pub fn attack_order(self) -> & 'static [Self] {
        match self {
            FleetFormation::Left => & [FleetFormation::Left, FleetFormation::Center, FleetFormation::Rear],
            FleetFormation::Center => & [FleetFormation::Center, FleetFormation::Rear],
            FleetFormation::Right => & [FleetFormation::Right, FleetFormation::Center, FleetFormation::Rear],
            FleetFormation::Rear => & [FleetFormation::Left, FleetFormation::Right, FleetFormation::Center, FleetFormation::Rear],
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
        let forms = [
            FleetFormation::Center,
            FleetFormation::Left,
            FleetFormation::Right,
            FleetFormation::Rear,
        ];

        for form in &forms {
            // a formation needs to have something to attack. if it doesnt then why is it there ?
            assert!(form.attack_order().len() > 0);

            // make sure we won't heal enemies by dealing negative damage
            for form2 in &forms {
                assert!(form.attack_coeff(form2) > 0.0);
            }
        }
    }
}
