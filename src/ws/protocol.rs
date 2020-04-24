use actix::Message;

#[derive(Debug)]
pub enum LobbyMessage {
    PlayerDisconnected,
    LobbyCreated,
}
impl Message for LobbyMessage {
    type Result = ();
}

