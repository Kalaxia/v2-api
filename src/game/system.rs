use uuid::Uuid;
use serde::{Serialize, Deserialize};
use std::collections::{HashMap};
use crate::game::{
    fleet::fleet::{FleetID, Fleet},
    game::MAP_SIZE,
    player::{PlayerID}
};

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct SystemID(Uuid);

#[derive(Serialize, Deserialize, Clone)]
pub struct System {
    pub id: SystemID,
    pub player: Option<PlayerID>,
    pub fleets: HashMap<FleetID, Fleet>,
    pub coordinates: Coordinates,
    pub unreachable: bool
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Coordinates {
    pub x: u8,
    pub y: u8
}

pub fn generate_systems() -> HashMap<SystemID, System> {
    let mut systems = HashMap::new();
    for y in 0..MAP_SIZE {
        for x in 0..MAP_SIZE {
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
        fleets: HashMap::new(),
        coordinates: Coordinates{ x, y },
        unreachable: false
    }
}