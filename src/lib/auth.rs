use serde::{Deserialize, Serialize};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use actix_web::dev::Payload;
use actix_web::{web::Data, FromRequest, HttpResponse, HttpRequest, Responder};
use crate::{AppState, lib::error::{ServerError, InternalError}, game::player::{Player, PlayerID}};
use futures::future::{ready, Ready, FutureExt, Future};
use std::{convert::{TryFrom, TryInto}, str::FromStr, default::Default, pin::Pin};


const JWT_SECRET: & 'static [u8] = b"secret";

/// This structure represent an HTTP authentication token.
/// Every route with a `Claim` in its parameters will only allow authentified requests.
#[derive(Serialize, Deserialize)]
pub struct Claims {
    pub pid:PlayerID,
}

/// Encode the String representation of a JWT Claims
impl TryFrom<Claims> for String {
    type Error = ServerError;

    fn try_from(claims: Claims) -> Result<Self, ServerError> {
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(JWT_SECRET),
        ).map_err(Into::into)
    }
}

/// Decode a JWT Claims from a String supposedly representing it
impl FromStr for Claims {
    type Err = ServerError;

    fn from_str(token: &str) -> Result<Self, ServerError> {
        decode::<Claims>(
            token,
            &DecodingKey::from_secret(JWT_SECRET),
            &Validation { validate_exp: false, ..Default::default() }
        ).map(|data| data.claims).map_err(Into::into)
    }
}

impl FromRequest for Claims {
    type Error = ServerError;
    type Future = Pin<Box<dyn Future<Output=Result<Self, ServerError>>>>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload) -> <Self as FromRequest>::Future {
        let db = req.app_data::<Data<AppState>>()
            .expect("No Data<PgPool> setup in server")
            .db_pool.clone();
        let claimsr = req
                .headers()
                .get("Authorization")
                .ok_or(InternalError::NoAuthorizationGiven.into())
                .and_then(|header| {
                    let token = header.to_str().unwrap().split(' ').last().unwrap();
                    Claims::from_str(token).map_err(Into::into)
                });

        async move {
            match claimsr {
                Ok(claim) => {
                    Player::find(claim.pid, &db).await.map(|_| claim).map_err(|e| {
                        e.into()
                    })
                },
                Err(e) => Err(e),
            }
        }.boxed_local()
    }
}

impl Responder for Claims {
    type Error = ServerError;
    type Future = Ready<Result<HttpResponse, ServerError>>;

    fn respond_to(self, _req: &HttpRequest) -> Self::Future {
        #[derive(Serialize)]
        struct TokenResponse { token: String };

        ready(self.try_into()
            .map(|token| HttpResponse::Ok().json(TokenResponse { token }))
            .map_err(Into::into))
    }
}
