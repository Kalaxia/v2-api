use serde::{Serialize, Deserialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone)]
pub struct Faction{
    pub id: FactionID,
}

#[derive(Serialize, Deserialize, Copy, Clone, Hash, PartialEq, Eq)]
pub struct FactionID(u8);

pub fn generate_factions() -> HashMap<FactionID, Faction> {
    let mut factions = HashMap::new();
    factions.insert(FactionID(1), Faction{ id: FactionID(1) });
    factions.insert(FactionID(2), Faction{ id: FactionID(2) });
    factions.insert(FactionID(3), Faction{ id: FactionID(3) });
    factions
}