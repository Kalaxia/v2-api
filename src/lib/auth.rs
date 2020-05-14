use serde::{Deserialize, Serialize};
use jsonwebtoken::{errors::Error as JwtError, decode, encode, DecodingKey, EncodingKey, Header, Validation};
use actix_web::dev::Payload;
use actix_web::{FromRequest, HttpResponse, HttpRequest, Responder};
use crate::{lib::error::{ServerError, InternalError}, game::player::PlayerID};
use futures::future::{ready, Ready};
use std::default::Default;

const JWT_SECRET: & 'static [u8] = b"secret";

/// This structure represent an HTTP authentication token.
/// Every route with a `Claim` in its parameters will only allow authentified requests.
#[derive(Serialize, Deserialize)]
pub struct Claims {
    pub pid:PlayerID,
}

impl FromRequest for Claims {
    type Error = ServerError;
    type Future = Ready<Result<Self, ServerError>>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload) -> <Self as FromRequest>::Future {
        ready(
            req
                .headers()
                .get("Authorization")
                .ok_or(InternalError::NoAuthorizationGiven.into())
                .and_then(|header| {
                    let token = header.to_str().unwrap().split(' ').last().unwrap();
                    decode_jwt(token).map_err(Into::into)
                })
        )
    }
}

impl Responder for Claims {
    type Error = ServerError;
    type Future = Ready<Result<HttpResponse, ServerError>>;

    fn respond_to(self, _req: &HttpRequest) -> Self::Future {
        #[derive(Serialize)]
        struct TokenResponse { token: String };

        ready(create_jwt(self)
            .map(|token| HttpResponse::Ok().json(TokenResponse { token }))
            .map_err(Into::into))
    }
}

pub fn create_jwt(claims: Claims) -> Result<String, JwtError> {
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(JWT_SECRET),
    )
}

pub fn decode_jwt(token: &str) -> Result<Claims, JwtError> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(JWT_SECRET),
        &Validation { validate_exp: false, ..Default::default() }
    ).map(|data| data.claims)
}
