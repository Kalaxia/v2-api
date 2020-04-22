use serde::{Deserialize, Serialize};
use jsonwebtoken::errors::{Error as JwtError, ErrorKind};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use actix_web::dev::Payload;
use actix_web::{Error, FromRequest, HttpRequest};
use crate::game::player::Player;
use futures::future::{ready, Ready};

const JWT_SECRET: [u8; 6] = *b"secret";

#[derive(Serialize, Deserialize)]
pub struct Claims {
    pub player: Player,
    pub exp: usize,
}

impl FromRequest for Claims {
    type Error = Error;
    type Future = Ready<Result<Self, Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload) -> <Self as FromRequest>::Future {
        let token = match req.headers().get("Authorization") {
            Some(header) => header.to_str().unwrap().split(' ').last().unwrap(),
            None => panic!("Not authorization header found")
        };
        ready(decode_jwt(token).map_err(|err| match *err.kind() {
            ErrorKind::InvalidToken => panic!("Token is invalid"), // Example on how to handle a specific error
            ErrorKind::InvalidIssuer => panic!("Issuer is invalid"), // Example on how to handle a specific error
            _ => panic!("Some other errors")
        }))
    }
}

pub fn create_jwt(claims: Claims) -> Result<String, JwtError> {
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(&JWT_SECRET),
    )
}

pub fn decode_jwt(token: &str) -> Result<Claims, JwtError> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(&JWT_SECRET),
        &Validation::default()
    ).map(|data| data.claims)
}