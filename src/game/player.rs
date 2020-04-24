use actix_web::{post, HttpResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::lib::{Result, auth};

#[derive(Serialize, Deserialize, Clone)]
pub struct Player {
    pub id: Uuid,
    pub username: String
}

#[post("/login")]
pub async fn login() -> Result<HttpResponse> {
    let player = Player {
        id: Uuid::new_v4(),
        username: String::from("")
    };
    #[derive(Serialize)]
    struct TokenResponse {
        token: String
    };
    auth::create_jwt(auth::Claims { player, exp: 10000000000 })
        .map(|token| HttpResponse::Ok().json(TokenResponse{ token }))
        .map_err(Into::into)
}
