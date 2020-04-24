//! `LobbyWebsocket` is an actor. It maintains list of connection client session.
//! And manages available rooms. Peers send messages to other peers in same
//! room through `LobbyWebsocket`.

use actix::prelude::*;
use std::collections::HashMap;
use uuid::Uuid;

/// Chat server sends this messages to session
#[derive(Message)]
#[rtype(result = "()")]
pub struct Message(pub String);

/// Message for chat server communications

/// New chat session is created
#[derive(Message)]
#[rtype(result = "Option<Uuid>")]
pub struct Connect {
    pub addr: Recipient<Message>,
}

/// Session is disconnected
#[derive(Message)]
#[rtype(result = "()")]
pub struct Disconnect {
    pub id: Uuid,
}

/// Send message to specific room
#[derive(Message)]
#[rtype(result = "()")]
pub struct ClientMessage {
    /// Id of the client session
    pub id: Uuid,
    /// Peer message
    pub msg: String,
}


/// `LobbyWebsocket` manages chat rooms and responsible for coordinating chat
/// session. implementation is super primitive
#[derive(Clone)]
pub struct LobbyWebsocket {
    sessions: HashMap<Uuid, Recipient<Message>>,
}

impl Default for LobbyWebsocket {
    fn default() -> LobbyWebsocket {
        LobbyWebsocket {
            sessions: HashMap::new(),
        }
    }
}

impl LobbyWebsocket {
    /// Send message to all users in the room
    fn send_message(&self, message: &str, skip_id: Option<Uuid>) {
        let excluded_id = match skip_id {
            Some(id) => id,
            None => Uuid::nil(),
        };
        for (id, recipient) in &self.sessions {
            if excluded_id != *id {
                recipient.do_send(Message(message.to_owned()));
            }
        }
    }
}

/// Make actor from `LobbyWebsocket`
impl Actor for LobbyWebsocket {
    /// We are going to use simple Context, we just need ability to communicate
    /// with other actors.
    type Context = Context<Self>;
}

/// Handler for Connect message.
///
/// Register new session and assign unique id to this session
impl Handler<Connect> for LobbyWebsocket {
    type Result = Option<Uuid>;

    fn handle(&mut self, msg: Connect, _: &mut Context<Self>) -> Self::Result {
        println!("Someone joined");

        // notify all users in same room
        self.send_message("Someone joined", None);

        // register session with random id
        let id = Uuid::new_v4();
        self.sessions.insert(id, msg.addr);

        Some(id)
    }
}

/// Handler for Disconnect message.
impl Handler<Disconnect> for LobbyWebsocket {
    type Result = ();

    fn handle(&mut self, msg: Disconnect, _: &mut Context<Self>) {
        println!("{} disconnected !", &msg.id);

        // remove address
        self.sessions.remove(&msg.id);
    }
}

/// Handler for Message message.
impl Handler<ClientMessage> for LobbyWebsocket {
    type Result = ();

    fn handle(&mut self, msg: ClientMessage, _: &mut Context<Self>) {
        self.send_message(msg.msg.as_str(), Some(msg.id));
    }
}