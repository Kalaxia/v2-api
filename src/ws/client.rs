use std::time::{Duration, Instant};

use actix::*;
use actix_web::{web, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use crate::{
    lib::{Result, error::InternalError, auth::Claims},
    game::{player::{PlayerID, PlayerData}},
    ws::protocol,
    AppState,
};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// How long before lack of client response causes a timeout
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

/// Entry point for our route
pub async fn entrypoint(
    req: HttpRequest,
    stream: web::Payload,
    state: web::Data<AppState>,
    claims: Claims,
) -> Result<HttpResponse> {
    let mut players = state.players.write().unwrap();
    let p = players.get_mut(&claims.pid);

    if p.is_none() {
        return Err(InternalError::PlayerUnknown.into());
    }
    let player = p.unwrap();
    let (addr, resp) = ws::start_with_addr(ClientSession{
        hb: Instant::now(),
        state: state.clone(),
        pid: player.data.id.clone()
    }, &req, stream)?;
    player.websocket = Some(addr);
    let data = player.data.clone();
    drop(players);

    state.ws_broadcast(&protocol::Message::<PlayerData>{
        action: protocol::Action::PlayerConnected,
        data: data.clone()
    }, Some(data.id.clone()), Some(true));

    Ok(resp)
}

pub struct ClientSession {
    hb: Instant,
    state: web::Data<AppState>,
    pid: PlayerID
}

impl ClientSession {
    fn logout(&self) {
        let mut players = self.state.players.write().unwrap();
        let data = players.get(&self.pid).unwrap().data.clone();
        players.remove(&self.pid);

        if data.lobby != None {
            let mut lobbies = self.state.lobbies.write().unwrap();
            let lobby = lobbies.get_mut(&data.clone().lobby.unwrap()).unwrap();
            lobby.players.remove(&self.pid);
            lobby.ws_broadcast(&players, &protocol::Message::<PlayerData>{
                action: protocol::Action::PlayerLeft,
                data: data.clone()
            }, Some(&self.pid));
        }
        drop(players);

        self.state.ws_broadcast(&protocol::Message::<PlayerData>{
            action: protocol::Action::PlayerDisconnected,
            data: data.clone()
        }, Some(self.pid), Some(true));
    }
}

impl Actor for ClientSession {
    type Context = ws::WebsocketContext<Self>;

    /// Method is called on actor start.
    /// We register ws session with LobbyWebsocket
    fn started(&mut self, ctx: &mut Self::Context) {
        // we'll start heartbeat process on session start.
        self.hb(ctx);
    }

    fn stopping(&mut self, _: &mut Self::Context) -> Running {
        self.logout();
        Running::Stop
    }
}

/// Handle messages from chat server, we simply send it to peer websocket
impl<T> Handler<protocol::Message<T>> for ClientSession
where
    T: Clone + Send + serde::Serialize {
    type Result = ();

    fn handle(&mut self, msg: protocol::Message<T>, ctx: &mut Self::Context)  {
        ctx.text(serde_json::to_string(&msg).unwrap())
    }
}

/// WebSocket message handler
impl StreamHandler<std::result::Result<ws::Message, ws::ProtocolError>> for ClientSession {
    fn handle(
        &mut self,
        msg: std::result::Result<ws::Message, ws::ProtocolError>,
        ctx: &mut Self::Context,
    ) {
        let msg = match msg {
            Err(_) => {
                ctx.stop();
                return;
            }
            Ok(msg) => msg,
        };

        println!("WEBSOCKET MESSAGE: {:?}", msg);
        match msg {
            ws::Message::Ping(msg) => {
                self.hb = Instant::now();
                ctx.pong(&msg);
            }
            ws::Message::Pong(_) => {
                self.hb = Instant::now();
            }
            ws::Message::Text(_text) => {
                
            }
            ws::Message::Binary(_) => println!("Unexpected binary"),
            ws::Message::Close(_) => {
                ctx.stop();
            }
            ws::Message::Continuation(_) => {
                ctx.stop();
            }
            ws::Message::Nop => (),
        };
    }
}

impl ClientSession {
    /// helper method that sends ping to client every second.
    ///
    /// also this method checks heartbeats from client
    fn hb(&self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            // check client heartbeats
            if Instant::now().duration_since(act.hb) > CLIENT_TIMEOUT {
                // heartbeat timed out
                println!("Websocket Client heartbeat failed, disconnecting!");

                // stop actor
                ctx.stop();

                // don't try to send a ping
                return;
            }

            ctx.ping(b"");
        });
    }
}
