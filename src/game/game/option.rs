use serde::{Serialize, Deserialize};
use galaxy_rs::GalaxyBuilder;


#[derive(Debug, Serialize, Deserialize, Copy, Clone, PartialEq, sqlx::Type)]
#[sqlx(rename = "VARCHAR")]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum GameOptionSpeed {
    Slow,
    Medium,
    Fast,
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, PartialEq, sqlx::Type)]
#[sqlx(rename = "VARCHAR")]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum GameOptionMapSize {
    VerySmall,
    Small,
    Medium,
    Large,
    VeryLarge,
}

impl GameOptionSpeed {
    pub fn into_coeff(self) -> f64 {
        match self {
            GameOptionSpeed::Slow => 1.2,
            GameOptionSpeed::Medium => 1.0,
            GameOptionSpeed::Fast => 0.8,
        }
    }

    pub fn into_travel_speed(self) -> f64 {
        match self  {
            GameOptionSpeed::Slow => 0.4,
            GameOptionSpeed::Medium => 0.55,
            GameOptionSpeed::Fast => 0.7,
        }
    }
}

impl GameOptionMapSize {
    pub fn to_galaxy_builder(&self) -> GalaxyBuilder {
        match self {
            GameOptionMapSize::VerySmall => GalaxyBuilder::default()
                .min_distance(Some(1.0))
                .cloud_population(1)
                .nb_arms(3)
                .nb_arm_bones(3)
                .slope_factor(0.6)
                .arm_slope(std::f64::consts::PI / 4.0)
                .arm_width_factor(1.0 / 32.0),
            GameOptionMapSize::Small => GalaxyBuilder::default()
                .min_distance(Some(1.0))
                .cloud_population(2)
                .nb_arms(3)
                .nb_arm_bones(5)
                .slope_factor(0.5)
                .arm_slope(std::f64::consts::PI / 4.0)
                .arm_width_factor(1.0 / 28.0),
            GameOptionMapSize::Medium => GalaxyBuilder::default()
                .min_distance(Some(1.0))
                .cloud_population(2)
                .nb_arms(4)
                .nb_arm_bones(10)
                .slope_factor(0.5)
                .arm_slope(std::f64::consts::PI / 2.0)
                .arm_width_factor(1.0 / 24.0),
            GameOptionMapSize::Large => GalaxyBuilder::default()
                .min_distance(Some(1.0))
                .cloud_population(2)
                .nb_arms(5)
                .nb_arm_bones(15)
                .slope_factor(0.4)
                .arm_slope(std::f64::consts::PI / 4.0)
                .arm_width_factor(1.0 / 20.0),
            GameOptionMapSize::VeryLarge => GalaxyBuilder::default()
                .min_distance(Some(1.5))
                .cloud_population(2)
                .nb_arms(6)
                .nb_arm_bones(20)
                .slope_factor(0.4)
                .arm_slope(std::f64::consts::PI / 4.0)
                .arm_width_factor(1.0 / 16.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_construction_time_coeff() {
        assert_eq!(1.2, GameOptionSpeed::Slow.into_coeff());
        assert_eq!(1.0, GameOptionSpeed::Medium.into_coeff());
        assert_eq!(0.8, GameOptionSpeed::Fast.into_coeff());
    }

    #[test]
    fn test_get_travel_speed() {
        assert_eq!(0.4, GameOptionSpeed::Slow.into_travel_speed());
        assert_eq!(0.55, GameOptionSpeed::Medium.into_travel_speed());
        assert_eq!(0.7, GameOptionSpeed::Fast.into_travel_speed());
    }
}