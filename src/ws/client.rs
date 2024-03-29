use std::time::{Duration, Instant};
use actix::*;
use actix_web::{web, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use futures::executor::block_on;
use crate::{
    lib::{
        Result,
        log::log,
        auth::Claims
    },
    game::{
        lobby::{ Lobby, LobbyAddClientMessage, LobbyRemoveClientMessage },
        game::{
            game::Game,
            server::{GameAddClientMessage, GameRemovePlayerMessage},
        },
        player::{Player, PlayerID},
    },
    ws::protocol,
    AppState,
};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// How long before lack of client response causes a timeout
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

/// Entry point for our the WebSocket handshake
pub async fn entrypoint(
    req: HttpRequest,
    stream: web::Payload,
    state: web::Data<AppState>,
    claims: Claims,
) -> Result<HttpResponse> {
    let player = Player::find(claims.pid, &state.db_pool).await?;
    // Creates the websocket client for the current player
    let (client, resp) = ws::start_with_addr(ClientSession{
        hb: Instant::now(),
        state: state.clone(),
        pid: player.id.clone()
    }, &req, stream)?;

    let mut missing_messages = state.missing_messages_mut();
    if let Some(player_messages) = missing_messages.get_mut(&player.id) {
        log(
            gelf::Level::Warning,
            "Player reconnected",
            &format!("{} has recover its websocket connection", player.username),
            vec![
                ("messages_number", player_messages.len().to_string()),
            ],
            &state.logger
        );

        for message in player_messages {
            client.do_send(message.clone());
        }

        missing_messages.remove(&player.id);
    }
    

    if let Some(lobby_id) = player.lobby {
        let lobbies = state.lobbies();
        let lobby_server = lobbies.get(&lobby_id).expect("Lobby server not found");

        lobby_server.send(LobbyAddClientMessage(player.id.clone(), client)).await?;
    } else if let Some(game_id) = player.game {
        let games = state.games();
        let game_server = games.get(&game_id).expect("Game server not found");

        game_server.send(GameAddClientMessage(player.id.clone(), client)).await?;
    } else {
        state.add_client(&player.id, client);
    }

    state.ws_broadcast(&protocol::Message::new(
        protocol::Action::PlayerConnected,
        player.clone(),
        Some(player.id.clone()),
    ));

    Ok(resp)
}

/// WebSocket actor used to communicate with a player.
pub struct ClientSession {
    hb: Instant,
    state: web::Data<AppState>,
    pid: PlayerID
}

impl ClientSession {
    async fn logout(&self) -> Result<()> {
        let player = Player::find(self.pid, &self.state.db_pool).await.unwrap();
        {
            let mut clients = self.state.clients_mut();
            clients.remove(&self.pid);
        };

        log(
            gelf::Level::Warning,
            "Player disconnected",
            &format!("{} has lost its websocket connection", player.username),
            vec![],
            &self.state.logger
        );

        if let Some(lobby_id) = player.lobby {
            let mut lobby = Lobby::find(lobby_id, &self.state.db_pool).await.unwrap();
            let lobbies = self.state.lobbies();
            let lobby_server = lobbies.get(&lobby.id).expect("Lobby server not found");
            let (_, is_empty) = std::sync::Arc::try_unwrap(lobby_server.send(LobbyRemoveClientMessage(player.id.clone())).await?).ok().unwrap();
            if is_empty {
                self.state.clear_lobby(lobby, player.id).await?;
            } else if player.id == lobby.owner {
                lobby.update_owner(&self.state.db_pool).await?;
                lobby_server.do_send(protocol::Message::new(
                    protocol::Action::LobbyOwnerUpdated,
                    lobby.owner.clone(),
                    None,
                ));
            }
        } else if let Some(game_id) = player.game {
            let mut games = self.state.games_mut();
            let game = games.get_mut(&game_id).expect("Game not found");

            let (_, is_empty) = std::sync::Arc::try_unwrap(game.send(GameRemovePlayerMessage(player.id.clone())).await?).ok().unwrap();
            if is_empty {
                drop(games);
                let game = Game::find(game_id, &self.state.db_pool).await?;
                self.state.clear_game(&game).await?;
            }
        }
        self.state.ws_broadcast(&protocol::Message::new(
            protocol::Action::PlayerDisconnected,
            player.clone(),
            Some(self.pid),
        ));
        Ok(())
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
        let res = block_on(self.logout());
        if res.is_err() {
            println!("Logout error : {:?}", res);
        }
        Running::Stop
    }
}

/// Handle messages from chat server, we simply send it to peer websocket
impl Handler<protocol::Message> for ClientSession {
    type Result = ();

    fn handle(&mut self, msg: protocol::Message, ctx: &mut Self::Context) -> Self::Result {
        if msg.skip_id == Some(self.pid) {
            return;
        }
        ctx.text(serde_json::to_string(&msg).expect("Couldnt serialize WsMessage data"))
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
    #[allow(clippy::unused_self)]
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
