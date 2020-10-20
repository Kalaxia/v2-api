use crate::{
    game::player::Player,
    lib::uuid::Uuid,
};

/// Tokens representing the type of WS message sent to notify a player.
#[derive(serde::Serialize, Clone)]
pub enum Action {
    BuildingConstructed,
    CombatEnded,
    FactionPointsUpdated,
    FleetCreated,
    FleetArrived,
    FleetSailed,
    FleetTransfer,
    GameStarted,
    LobbyCreated,
    LobbyOptionsUpdated,
    LobbyOwnerUpdated,
    LobbyNameUpdated,
    LobbyRemoved,
    LobbyLaunched,
    PlayerConnected,
    PlayerJoined,
    PlayerUpdate,
    PlayerMoneyTransfer,
    PlayerLeft,
    PlayerDisconnected,
    PlayerIncome,
    ShipQueueFinished,
    SystemConquerred,
    SystemsCreated,
    Victory,
}

#[derive(actix::Message, serde::Serialize, Clone)]
#[rtype(result = "()")]
pub struct Message {
    pub action: Action,
    pub data: serde_json::Value,
    pub skip_id: Option<Uuid<Player>>,
}

impl Message {
  pub fn new<T : serde::Serialize>(action: Action, data: T, skip_id: Option<Uuid<Player>>) -> Self {
    Self {
      action,
      data: serde_json::value::to_value(data).unwrap(),
      skip_id,
    }
  }
}
