use crate::game::player::player::PlayerID;

/// Tokens representing the type of WS message sent to notify a player.
#[derive(serde::Serialize, Clone, Debug)]
#[non_exhaustive]
pub enum Action {
    BuildingConstructed,
    BattleStarted,
    BattleEnded,
    ConquestCancelled,
    ConquestStarted,
    ConquestUpdated,
    FactionPointsUpdated,
    FleetCreated,
    FleetArrived,
    FleetSailed,
    FleetTransfer,
    FleetJoinedBattle,
    GameStarted,
    LobbyCreated,
    LobbyOptionsUpdated,
    LobbyOwnerUpdated,
    LobbyNameUpdated,
    LobbyRemoved,
    LobbyLaunched,
    NewChatMessage,
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

#[derive(actix::Message, serde::Serialize, Clone, Debug)]
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
