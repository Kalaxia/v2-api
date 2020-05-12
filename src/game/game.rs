use crate::{
    lib::error::{InternalError},
    game::{
        lobby::Lobby,
        player::{PlayerID, Player},
        system::{SystemID, System, generate_systems}
    },
    ws::protocol
}
;
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use std::collections::{HashMap, HashSet};
use actix::prelude::*;
use std::time::Duration;

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct GameID(Uuid);

#[derive(Serialize, Deserialize, Clone)]
pub struct GameData {
    pub id: GameID,
    systems: HashMap<SystemID, System>
}

pub struct Game {
    players: HashMap<PlayerID, Player>,
    data: GameData
}

impl Game {
    fn begin(&self) {
        self.ws_broadcast(&protocol::Message::<GameData>{
            action: protocol::Action::GameStarted,
            data: self.data.clone()
        }, None);
    }

    fn ws_broadcast<T: 'static>(
        &self,
        message: &protocol::Message<T>,
        skip_id: Option<&PlayerID>
    ) where
        T: Clone + Send + Serialize
    {
        for (id, player) in self.players.iter() {
            if Some(id) != skip_id {
                player.websocket.as_ref().map(|ws| {
                    ws.do_send(message.clone());
                });
            }
        }
    }
}

impl Actor for Game {
    type Context = Context<Self>;
    
    fn started(&mut self, ctx: &mut Context<Self>) {
        self.ws_broadcast(&protocol::Message::<GameData>{
            action: protocol::Action::LobbyLaunched,
            data: self.data.clone()
        }, None);
        ctx.run_later(Duration::new(5, 0), |this, _| this.begin());
    }
 
    fn stopped(&mut self, ctx: &mut Context<Self>) {
        println!("Game is stopped");
    }
}

pub fn create_game(lobby: &Lobby, players: &mut HashMap<PlayerID, Player>) -> (GameID, Addr<Game>) {
    let mut game = Game{
        players: HashMap::new(),
        data: GameData{
            id: GameID(Uuid::new_v4()),
            systems: generate_systems()
        }
    };
    lobby.players.iter().for_each(|pid| players
        .get_mut(&pid)
        .ok_or(InternalError::PlayerUnknown)
        .and_then(|p| {
            p.data.lobby = None;
            p.data.game = Some(game.data.id.clone());
            game.players.insert(p.data.id.clone(), p.clone());
            return Ok(())
        }).unwrap()
    );
    (game.data.id.clone(), game.start())
}
