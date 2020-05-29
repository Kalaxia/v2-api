use jsonwebtoken::errors::{Error as JwtError};
use actix_web::{http::StatusCode, Error as ActixWebError, ResponseError};
use actix_web_actors::ws::ProtocolError;
use actix::MailboxError;
use std::fmt::{Display, Formatter, Error as FmtError};

/// This is the global server error type implemented as a convenient wrapper around all kind of
/// errors we could encounter using externam libraries.
///
/// Please, try tu use this type of error instead of specific ones at least at the front-end of the
/// server, as it will be updated to handle more error cases as we add more libraries or more
/// crate-specific errors.
#[derive(Debug)]
pub enum ServerError {
    ActixWebError(ActixWebError),
    ActixWSError(ProtocolError),
    JwtError(JwtError),
    InternalError(InternalError),
    MailboxError(MailboxError)
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

impl Display for ServerError {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FmtError> {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for ServerError {}

impl ResponseError for ServerError {
    fn status_code(&self) -> StatusCode {

        use InternalError::*;

        println!("{:?}", self);
        match self {
            ServerError::ActixWebError(e) => e.as_response_error().status_code(),
            ServerError::JwtError(_) => StatusCode::UNAUTHORIZED,
            ServerError::InternalError(e) => match e {
                NoAuthorizationGiven => StatusCode::UNAUTHORIZED,
                AccessDenied => StatusCode::FORBIDDEN,
                AlreadyInLobby | NotInLobby | NotEnoughMoney | FleetInvalidDestination | FleetAlreadyTravelling => StatusCode::CONFLICT,
                PlayerUnknown | LobbyUnknown | FleetUnknown | GameUnknown | SystemUnknown => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            ServerError::ActixWSError(e) => e.status_code(),
            ServerError::MailboxError(_) => StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

/// This enum represent all kinds of errors this specific server can encounter.
#[derive(Debug)]
pub enum InternalError {
    /// A player tried to perform a restricted operation
    AccessDenied,
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
    /// A Claims was requested by the route but none were given
    NoAuthorizationGiven,
    /// A player tried to spend an unauthorized amount of money
    NotEnoughMoney,
}
