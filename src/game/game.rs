use crate::game::{
    player::{PlayerID},
    system::{SystemID, System, generate_systems}
}
;
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use std::collections::{HashMap, HashSet};

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct GameID(Uuid);

#[derive(Serialize, Deserialize, Clone)]
pub struct Game {
    pub id: GameID,
    players: HashSet<PlayerID>,
    systems: HashMap<SystemID, System>
}

pub fn create_game(players: HashSet<PlayerID>) -> Game {
    Game{
        id: GameID(Uuid::new_v4()),
        players,
        systems: generate_systems()
    }
}
