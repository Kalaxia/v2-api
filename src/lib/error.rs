use jsonwebtoken::errors::{Error as JwtError};
use actix_web::{http::StatusCode, Error as ActixWebError, ResponseError, HttpResponse};
use actix_web_actors::ws::ProtocolError;
use actix::MailboxError;
use std::fmt::{Display, Formatter, Error as FmtError};
use sqlx_core::{Error as SqlxError};
use serde::Serialize;
use uuid::{Error as UuidError};

/// This is the global server error type implemented as a convenient wrapper around all kind of
/// errors we could encounter using externam libraries.
///
/// Please, try tu use this type of error instead of specific ones at least at the front-end of the
/// server, as it will be updated to handle more error cases as we add more libraries or more
/// crate-specific errors.
#[derive(Debug, Serialize)]
#[non_exhaustive]
#[serde(tag = "type")]
pub enum ServerError {
    ActixWebError(
        #[serde(skip_serializing)]
        ActixWebError
    ),
    ActixWSError(
        #[serde(skip_serializing)]
        ProtocolError
    ),
    JwtError(
        #[serde(skip_serializing)]
        JwtError
    ),
    InternalError(
        #[serde(rename(serialize = "reason"))]
        InternalError
    ),
    MailboxError(
        #[serde(skip_serializing)]
        MailboxError
    ),
    SqlxError(
        #[serde(skip_serializing)]
        SqlxError
    ),
    UuidError(
        #[serde(skip_serializing)]
        UuidError
    )
}

impl ServerError {
    pub fn if_row_not_found<E>(internal_error:E) -> impl FnOnce(SqlxError) -> Self
        where E : Into<Self>,
    {
        |e| {
            match e {
                SqlxError::RowNotFound => internal_error.into(),
                _ => e.into()
            }
        }
    }
}

impl From<ActixWebError> for ServerError {
    fn from(error:ActixWebError) -> Self { Self::ActixWebError(error) }
}

impl From<JwtError> for ServerError {
    fn from(error:JwtError) -> Self { Self::JwtError(error) }
}

impl From<InternalError> for ServerError {
    fn from(error:InternalError) -> Self { Self::InternalError(error) }
}

impl From<ProtocolError> for ServerError {
    fn from(error:ProtocolError) -> Self { Self::ActixWSError(error) }
}

impl From<MailboxError> for ServerError {
    fn from(error:MailboxError) -> Self { Self::MailboxError(error) }
}

impl From<SqlxError> for ServerError {
    fn from(error:SqlxError) -> Self { Self::SqlxError(error) }
}

impl From<UuidError> for ServerError {
    fn from(error:UuidError) -> Self { Self::UuidError(error) }
}

impl Display for ServerError {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FmtError> {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for ServerError {}

impl ResponseError for ServerError {
    #[allow(unreachable_patterns)]
    fn status_code(&self) -> StatusCode {

        use InternalError::*;

        println!("{:?}", self);
        match self {
            ServerError::ActixWebError(e) => e.as_response_error().status_code(),
            ServerError::JwtError(_) => StatusCode::UNAUTHORIZED,
            ServerError::InternalError(e) => match e {
                NoAuthorizationGiven => StatusCode::UNAUTHORIZED,
                AccessDenied => StatusCode::FORBIDDEN,
                Conflict | AlreadyInLobby | NotInLobby | NotEnoughMoney | FleetInvalidDestination | FleetAlreadyTravelling | FleetEmpty | PlayerUsernameAlreadyTaken => StatusCode::CONFLICT,
                NotFound | FactionUnknown | PlayerUnknown | LobbyUnknown | FleetUnknown | GameUnknown | SystemUnknown => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            ServerError::ActixWSError(e) => e.status_code(),
            ServerError::MailboxError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ServerError::SqlxError(e) => match e {
                SqlxError::RowNotFound => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            ServerError::UuidError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code())
            .json(self)
    }
}

/// This enum represent all kinds of errors this specific server can encounter.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub enum InternalError {
    /// A player tried to perform a restricted operation
    AccessDenied,
    /// A required data does not exist
    NotFound,
    /// The requested operation conflicts with data
    Conflict,
    /// We couldn't map a FactionID to an existing faction
    FactionUnknown,
    /// We couldn't map a PlayerID to an existing player
    PlayerUnknown,
    /// We couldn't map a FleetID to an existing fleet
    FleetUnknown,
    /// We couldn't map a GameID to an existing game
    GameUnknown,
    /// We couldn't map a LobbyID to an existing Lobby
    LobbyUnknown,
    /// We couldn't map a SystemID to an existing System
    SystemUnknown,
    /// A player already in a lobby tries to create a lobby
    AlreadyInLobby,
    /// A player wants to modify a lobby its not in
    NotInLobby,
    /// A player wants to move a fleet to an invalid location
    FleetInvalidDestination,
    /// A player wants to move a fleet which is already on a journey
    FleetAlreadyTravelling,
    /// A player tried to move an empty fleet
    FleetEmpty,
    /// A player tried to take a username already taken by another in the same lobby
    PlayerUsernameAlreadyTaken,
    /// A Claims was requested by the route but none were given
    NoAuthorizationGiven,
    /// A player tried to spend an unauthorized amount of money
    NotEnoughMoney,
}
