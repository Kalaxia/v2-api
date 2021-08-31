use actix_web::{post, web, HttpResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::{
    lib::{
        Result,
        error::{ServerError, InternalError},
        time::Time,
        log::{log, Loggable},
        auth::Claims
    },
    game::{
        faction::FactionID,
        game::game::GameID,
        game::server::GameNotifyFactionMessage,
        player::player::{Player, PlayerID},
        system::system::{System, SystemID},
        fleet::squadron::{FleetSquadron},
    },
    ws::protocol,
    AppState
};
use sqlx::{PgPool, postgres::{PgRow, PgQueryAs}, FromRow, Executor, Error, Postgres};
use sqlx_core::row::Row;

#[derive(Serialize, Debug, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct ChatMessageID(pub Uuid);

#[derive(Debug, Serialize, Clone)]
pub struct ChatMessage{
    pub id: ChatMessageID,
    pub author: PlayerID,
    pub faction: FactionID,
    pub content: String,
    pub created_at: Time,
}

#[derive(Deserialize)]
pub struct ChatMessageRequest {
    pub content: String
}

impl From<ChatMessageID> for Uuid {
    fn from(cmid: ChatMessageID) -> Self { cmid.0 }
}

impl<'a> FromRow<'a, PgRow<'a>> for ChatMessage {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(ChatMessage {
            id: row.try_get("id").map(ChatMessageID)?,
            author: row.try_get("author_id").map(PlayerID)?,
            faction: row.try_get::<i32, _>("author_id").map(|fid| FactionID(fid as u8))?,
            content: row.try_get("content")?,
            created_at: row.try_get("created_at")?,
        })
    }
}

impl ChatMessage {
    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO communication__chat__messages(id, author_id, faction_id, content, created_at) VALUES($1, $2, $3, $4, $5)")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.author))
            .bind(i32::from(self.faction))
            .bind(self.content.clone())
            .bind(self.created_at)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
}

#[post("/send/")]
pub async fn send_message(
    state: web::Data<AppState>,
    info: web::Path<(GameID,)>,
    json_data: web::Json<ChatMessageRequest>,
    claims: Claims
) -> Result<HttpResponse> {
    let player = Player::find(claims.pid, &state.db_pool).await?;
    let faction_id = player.faction.ok_or(InternalError::FactionUnknown)?;

    let chat_message = ChatMessage{
        id: ChatMessageID(Uuid::new_v4()),
        author: player.id,
        faction: faction_id.clone(),
        content: json_data.content.clone(),
        created_at: Time::now(),
    };
    chat_message.insert(&mut &state.db_pool).await?;

    let games = state.games();
    let game = games.get(&info.0).cloned().ok_or(InternalError::GameUnknown)?;
    game.do_send(GameNotifyFactionMessage(faction_id, protocol::Message::new(
        protocol::Action::NewChatMessage,
        chat_message,
        None,
    )));

    Ok(HttpResponse::NoContent().finish())
}