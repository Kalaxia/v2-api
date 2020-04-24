use std::time::{Duration, Instant};

use actix::*;
use actix_web::{web, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use crate::{
    lib::{Result, error::InternalError, auth::Claims},
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

    match players.get_mut(&claims.pid) {
        Some(player) => {
            let (addr, resp) = ws::start_with_addr(ClientSession{ hb:Instant::now() }, &req, stream)?;
            player.websocket = Some(addr);
            Ok(resp)
        },
        None => Err(InternalError::PlayerUnknown.into())
    }
}

pub struct ClientSession { hb:Instant }

impl Actor for ClientSession {
    type Context = ws::WebsocketContext<Self>;

    /// Method is called on actor start.
    /// We register ws session with LobbyWebsocket
    fn started(&mut self, ctx: &mut Self::Context) {
        // we'll start heartbeat process on session start.
        self.hb(ctx);
    }

    fn stopping(&mut self, _: &mut Self::Context) -> Running {
        Running::Stop
    }
}

/// Handle messages from chat server, we simply send it to peer websocket
impl Handler<protocol::LobbyMessage> for ClientSession {
    type Result = ();

    fn handle(&mut self, msg: protocol::LobbyMessage, ctx: &mut Self::Context) {
        ctx.text(format!("{:?}", msg))
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
