use serde::Serialize;

#[derive(Serialize, Clone)]
pub enum Action {
    GameStarted,
    LobbyCreated,
    LobbyUpdated,
    LobbyNameUpdated,
    LobbyRemoved,
    LobbyLaunched,
    PlayerConnected,
    PlayerJoined,
    PlayerUpdate,
    PlayerLeft,
    PlayerDisconnected
}

#[derive(actix::Message, Serialize, Clone)]
#[rtype(result = "()")]
pub struct Message<T> {
    pub action: Action,
    pub data: T
}