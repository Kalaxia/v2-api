use actix::Message;
use serde::{Serialize, Deserialize};
use crate::game::{
    lobby::Lobby,
    player::PlayerID,
    player::PlayerData
};

#[derive(Serialize)]
pub enum Action {
    LobbyCreated,
    LobbyUpdated,
    PlayerJoined,
    PlayerDisconnected
}

#[derive(Message, Serialize)]
#[rtype(result = "()")]
pub struct PlayerMessage {
    pub action: Action,
    pub player: PlayerData
}

#[derive(Message, Serialize)]
#[rtype(result = "()")]
pub struct LobbyMessage{
    pub action: Action,
    pub lobby: Lobby
}