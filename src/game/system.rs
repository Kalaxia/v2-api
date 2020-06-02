use uuid::Uuid;
use serde::{Serialize, Deserialize};
use std::collections::{HashMap};
use crate::{
    game::{
        fleet::combat,
        fleet::fleet::{FleetID, Fleet},
        game::MAP_SIZE,
        player::{PlayerID, Player}
    },
    ws::protocol
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

#[derive(Serialize, Clone)]
pub struct ConquestData {
    pub system: System,
    pub fleet: Fleet,
}

#[derive(Clone)]
pub enum FleetArrivalOutcome {
    Conquerred{
        system: System,
        fleet: Fleet,
    },
    Defended{
        system: System,
        fleets: HashMap<FleetID, Fleet>,
    },
    Arrived{
        fleet: Fleet,
    }
}

impl System {
    /// Retrieve the present fleets by filtering the travelling ones
    pub fn get_fleets(&self) -> HashMap<FleetID, Fleet> {
        let mut fleets = HashMap::new();
        self.fleets
            .iter()
            .filter(|(_, f)| !f.is_travelling())
            .for_each(|(fid, f)| {
                fleets.insert(fid.clone(), f.clone());
            }); 
        fleets
    }

    pub fn resolve_fleet_arrival(&mut self, mut fleet: Fleet, player: &Player, system_owner: Option<&Player>) -> FleetArrivalOutcome {
        match system_owner {
            Some(system_owner) => {
                // Both players have the same faction, the arrived fleet just parks here
                if system_owner.data.faction == player.data.faction {
                    fleet.change_system(self);
                    return FleetArrivalOutcome::Arrived{ fleet };
                }
                // Conquest of the system by the arrived fleet
                let mut fleets = self.get_fleets();
                if fleets.is_empty() || combat::engage(&mut fleet, &mut fleets) == true {
                    return self.conquer(fleet);
                }
                fleets.insert(fleet.id.clone(), fleet.clone());
                FleetArrivalOutcome::Defended{ fleets, system: self.clone() }
            },
            None => self.conquer(fleet)
        }
    }

    pub fn conquer(&mut self, mut fleet: Fleet) -> FleetArrivalOutcome {
        // Clean defeated defenders fleets
        self.fleets.retain(|_, f| f.is_travelling()); 
        fleet.change_system(self);
        self.player = Some(fleet.player.clone());
        FleetArrivalOutcome::Conquerred{
            system: self.clone(),
            fleet: fleet,
        }
    }
}

impl From<FleetArrivalOutcome> for protocol::Message {
    fn from(outcome: FleetArrivalOutcome) -> Self {
        match outcome {
            FleetArrivalOutcome::Conquerred { system, fleet } => protocol::Message::new(
                protocol::Action::SystemConquerred,
                ConquestData{
                    system: system.clone(),
                    fleet: fleet.clone(),
                },
            ),
            FleetArrivalOutcome::Defended { system, fleets } => protocol::Message::new(
                protocol::Action::CombatEnded,
                combat::CombatData {
                    system: system.clone(),
                    fleets: fleets.clone(),
                },
            ),
            FleetArrivalOutcome::Arrived { fleet } => protocol::Message::new(
                protocol::Action::FleetArrived,
                fleet.clone(),
            )
        }
    }
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