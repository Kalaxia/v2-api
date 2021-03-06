use actix_web::{get, HttpResponse};
use chrono::{DateTime, Duration, Utc};
use serde::{Serialize, Deserialize};
use crate::{
    lib::{
        Result,
        time::Time,
    },
    game::game::option::GameOptionSpeed,
};

#[derive(Serialize, Copy, Clone)]
pub struct ShipModel {
    pub category: ShipModelCategory,
    pub strength: u16,
    pub construction_time: u16,
    pub cost: u16,
    pub damage: u16,
    pub combat_speed: u16,
    pub hit_points: u16,
    pub precision: u16,
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, PartialEq, sqlx::Type)]
#[sqlx(rename = "VARCHAR")]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum ShipModelCategory {
    Fighter,
    Corvette,
    Frigate,
    Cruiser,
}

impl ShipModelCategory {
    pub const fn to_data(self) -> ShipModel {
        match self {
            ShipModelCategory::Fighter => ShipModel{
                category: ShipModelCategory::Fighter,
                construction_time: 400,
                strength: 1,
                cost: 20,
                damage: 15,
                combat_speed: 80,
                hit_points: 10,
                precision: 60,
            },
            ShipModelCategory::Corvette => ShipModel{
                category: ShipModelCategory::Corvette,
                construction_time: 1500,
                strength: 10,
                cost: 140,
                damage: 40,
                combat_speed: 50,
                hit_points: 60,
                precision: 45,
            },
            ShipModelCategory::Frigate => ShipModel{
                category: ShipModelCategory::Frigate,
                construction_time: 2000,
                strength: 25,
                cost: 250,
                damage: 25,
                combat_speed: 35,
                hit_points: 100,
                precision: 50,
            },
            ShipModelCategory::Cruiser => ShipModel{
                category: ShipModelCategory::Cruiser,
                construction_time: 7000,
                strength: 75,
                cost: 600,
                damage: 80,
                combat_speed: 20,
                hit_points: 200,
                precision: 45,
            }
        }
    }
}

impl ShipModel {
    pub fn compute_construction_deadline(self, quantity: u16, from: Time, game_speed: GameOptionSpeed) -> Time {
        let datetime: DateTime<Utc> = from.into();

        Time(datetime.checked_add_signed(self.into_duration(quantity, game_speed)).expect("Could not add construction time"))
    }

    pub fn into_duration(self, quantity: u16, game_speed: GameOptionSpeed) -> Duration {
        Duration::milliseconds((
            (quantity as usize * self.construction_time as usize) as f64 * game_speed.into_coeff()
        ).ceil() as i64)
    }
}



#[get("/ship-models/")]
pub async fn get_ship_models() -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(vec![
        ShipModelCategory::Fighter.to_data(),
        ShipModelCategory::Corvette.to_data(),
        ShipModelCategory::Frigate.to_data(),
        ShipModelCategory::Cruiser.to_data(),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ship_model_data() {
        let fighter = ShipModelCategory::Fighter.to_data();

        assert_eq!(fighter.category, ShipModelCategory::Fighter);

        let cruiser = ShipModelCategory::Cruiser.to_data();

        assert_eq!(cruiser.category, ShipModelCategory::Cruiser);

        assert_ne!(fighter.cost, cruiser.cost);
    }

    #[test]
    fn test_ship_model_construction_milliseconds() {
        let fighter_model = ShipModelCategory::Fighter.to_data();

        assert_eq!(960, fighter_model.into_duration(2, GameOptionSpeed::Slow).num_milliseconds());
        assert_eq!(800, fighter_model.into_duration(2, GameOptionSpeed::Medium).num_milliseconds());
        assert_eq!(640, fighter_model.into_duration(2, GameOptionSpeed::Fast).num_milliseconds());
    }
}
