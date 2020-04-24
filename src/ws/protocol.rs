use actix::Message;

#[derive(Debug)]
pub enum LobbyMessage {
    PlayerDisconnected,
}
impl Message for LobbyMessage {
    type Result = ();
}

