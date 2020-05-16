use actix_web::{get, web, HttpResponse};
use actix::prelude::*;
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use std::sync::Arc;
use crate::{
    lib::{Result, error::InternalError},
    game::{
        faction::{FactionID},
        lobby::Lobby,
        player::{PlayerID, Player, PlayerData},
        system::{SystemID, System, generate_systems}
    },
    ws::protocol,
    AppState,
};

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

pub const MAP_SIZE: u8 = 10;

impl Game {
    fn init(&mut self) {
        self.data.systems = generate_systems();
        self.assign_systems();
    }

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

    fn assign_systems(&mut self) {
        let mut faction_players = HashMap::new();
        // @TODO: I think we can call directly here self.assign_system but we must avoid mutable borrow
        //     ``i`` has no current use (because the systems are not in order)
        //     and the faction sorting does not decrease the complexity of assign_system()
        //     so we could just assign the system when looping through the players
        // Regroup players per faction
        for (_, p) in self.players.iter() {
            let fid = p.data.faction.unwrap().clone();
            if !faction_players.contains_key(&fid) {
                faction_players.insert(fid, HashSet::new());
            }
            faction_players.get_mut(&fid).unwrap().insert(p.data.clone());
        }

        for (fid, players) in faction_players.iter() {
            let mut i = 0;
            for player in players.iter() {
                self.assign_system(*fid, i, player);
                i += 1;
            }
        }
    }

    fn assign_system(&mut self, fid: FactionID, i: u8, player: &PlayerData) {
        // Each faction is associated to a side of the grid
        let coordinates_check: Box<dyn Fn(u8, u8, u8) -> bool> = match fid {
            FactionID(1) => Box::new(|i, x, y| x > 0 || y < i),
            FactionID(2) => Box::new(|i, x, y| x < MAP_SIZE - 1 || y < i),
            FactionID(3) => Box::new(|i, x, y| y > 0 || x < i),
            FactionID(4) => Box::new(|i, x, y| y < MAP_SIZE - 1 || x < i),
            _ => return
        };
        for (_, system) in self.data.systems.iter_mut() {
            if coordinates_check(i, system.coordinates.x, system.coordinates.y) || system.player != None {
                continue;
            }
            println!("System ({:?}, {:?}) is affected to player {:?} of faction {:?} at loop {:?}", system.coordinates.x, system.coordinates.y, player.username.clone(), player.faction.clone(), i);
            system.player = Some(player.id.clone());
            return;
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
        ctx.run_later(Duration::new(1, 0), |this, _| this.init());
        ctx.run_later(Duration::new(5, 0), |this, _| this.begin());
    }
 
    fn stopped(&mut self, _ctx: &mut Context<Self>) {
        println!("Game is stopped");
    }
}

#[derive(Serialize, Clone)]
pub struct GameDataMessage{
    field: String
}

impl actix::Message for GameDataMessage {
    type Result = Arc<Vec<PlayerData>>;
}

impl Handler<GameDataMessage> for Game {
    type Result = Arc<Vec<PlayerData>>;

    fn handle(&mut self, msg: GameDataMessage, _ctx: &mut Self::Context) -> Self::Result {
        match msg.field.as_str() {
            "players" => Arc::new(self.players.iter().map(|(_, p)| p.data.clone()).collect::<Vec<PlayerData>>()),
            _ => Arc::new(Vec::new())
        }
    }
}

pub fn create_game(lobby: &Lobby, players: &mut HashMap<PlayerID, Player>) -> (GameID, Addr<Game>) {
    let mut game = Game{
        players: HashMap::new(),
        data: GameData{
            id: GameID(Uuid::new_v4()),
            systems: HashMap::new()
        }
    };
    for pid in &lobby.players {
        players.get_mut(pid).map(|p| {
            p.data.lobby = None;
            p.data.game = Some(game.data.id);
            game.players.insert(p.data.id, p.clone())
        });
    }
    (game.data.id.clone(), game.start())
}

#[get("/{id}/players/")]
pub async fn get_players(state: web::Data<AppState>, info: web::Path<(GameID,)>) -> Result<HttpResponse> {
    let games = state.games();
    println!("{:?}", games.iter().count());
    let game = games.get(&info.0).ok_or(InternalError::GameUnknown)?;
    match game.send(GameDataMessage{ field: String::from("players") }).await {
        Ok(data) => Ok(HttpResponse::Ok().json((*data).clone())),
        _ => Ok(HttpResponse::InternalServerError().finish())
    }
}