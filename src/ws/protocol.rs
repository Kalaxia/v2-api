use crate::game::player::PlayerID;

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
    LobbyOwnerUpdated,
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
    SystemsCreated,
    Victory,
}

#[derive(actix::Message, serde::Serialize, Clone)]
#[rtype(result = "()")]
pub struct Message {
    pub action: Action,
    pub data: serde_json::Value,
    pub skip_id: Option<PlayerID>,
}

impl Message {
  pub fn new<T : serde::Serialize>(action: Action, data: T, skip_id: Option<PlayerID>) -> Self {
    Self {
      action,
      data: serde_json::value::to_value(data).unwrap(),
      skip_id,
    }
  }
}