use uuid::Uuid;
use serde::{Serialize, Deserialize};
use std::collections::{HashMap};
use crate::game::player::{PlayerID};

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct SystemID(Uuid);

#[derive(Serialize, Deserialize, Clone)]
pub struct System {
    id: SystemID,
    player: Option<PlayerID>,
    coordinates: Coordinates,
    unreachable: bool
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Coordinates {
    x: u8,
    y: u8
}

const MAP_SIZE: u8 = 10;

pub fn generate_systems() -> HashMap<SystemID, System> {
    let mut systems = HashMap::new();
    for y in 1..MAP_SIZE {
        for x in 1..MAP_SIZE {
            let system = generate_system(x, y);
            systems.insert(system.id.clone(), system);
        }
    }
    systems
}

fn generate_system(x: u8, y: u8) -> System {
    System{
        id: SystemID(Uuid::new_v4()),
        player: None,
        coordinates: Coordinates{ x, y },
        unreachable: false
    }
}