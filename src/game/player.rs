use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct Player {
    id: usize,
    username: String
}

pub fn register_player() -> Player {
    Player {
        id: 1,
        username: String::from("")
    }
}