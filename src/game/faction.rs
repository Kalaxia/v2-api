use uuid::Uuid;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct Faction{
    id: FactionID,
}

#[derive(Serialize, Deserialize, Copy, Clone, Hash, PartialEq, Eq)]
pub struct FactionID(Uuid);