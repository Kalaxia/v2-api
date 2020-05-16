use serde::Serialize;

/// Tokens representing the type of WS message sent to notify a player.
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

/// This structure is generic over `T` to allow us to freely change the `T` sent for each message.
/// As long as `T` is `Serialize`, we can send whatever we want. It is up to the client to handle
/// the deserialization given the documented structure of the data sent.
#[derive(actix::Message, Serialize, Clone)]
#[rtype(result = "()")]
pub struct Message<T> {
    pub action: Action,
    pub data: T
}
