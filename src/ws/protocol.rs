use serde::Serialize;

#[derive(Serialize, Clone)]
pub enum Action {
    LobbyCreated,
    LobbyUpdated,
    LobbyNameUpdated,
    LobbyRemoved,
    PlayerJoined,
    PlayerUpdate,
    PlayerDisconnected
}

#[derive(actix::Message, Serialize, Clone)]
#[rtype(result = "()")]
pub struct Message<T> {
    pub action: Action,
    pub data: T
}