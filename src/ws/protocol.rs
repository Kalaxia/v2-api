/// Tokens representing the type of WS message sent to notify a player.
#[derive(serde::Serialize, Clone)]
pub enum Action {
    CombatEnded,
    FleetCreated,
    FleetArrived,
    FleetSailed,
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
    PlayerDisconnected,
    PlayerIncome,
    SystemConquerred,
}

#[derive(actix::Message, serde::Serialize, Clone)]
#[rtype(result = "()")]
pub struct Message {
    pub action: Action,
    pub data: serde_json::Value
}

impl Message {
  pub fn new<T : serde::Serialize>(action: Action, data: T) -> Self {
    Self {
      action,
      data: serde_json::value::to_value(data).unwrap(),
    }
  }
}