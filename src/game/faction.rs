use actix_web::{get, web, HttpResponse};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use crate::{
    AppState
};

#[derive(Serialize, Deserialize, Clone)]
pub struct Faction{
    pub id: FactionID,
    name: FactionName,
    color: (u8, u8, u8)
}

#[derive(Serialize, Deserialize, Clone)]
pub enum FactionName{
    Kalankar,
    Valkar,
    Adranite
}

#[derive(Serialize, Deserialize, Copy, Clone, Hash, PartialEq, Eq, Debug)]
pub struct FactionID(pub u8);

pub fn generate_factions() -> HashMap<FactionID, Faction> {
    let mut factions = HashMap::new();
    factions.insert(FactionID(1), Faction{
        id: FactionID(1),
        name: FactionName::Kalankar,
        color: (0,255,255)
    });
    factions.insert(FactionID(2), Faction{
        id: FactionID(2),
        name: FactionName::Valkar,
        color: (0,0,255)
    });
    factions.insert(FactionID(3), Faction{
        id: FactionID(3),
        name: FactionName::Adranite,
        color: (255,0,0)
    });
    factions
}

#[get("/")]
pub async fn get_factions(state: web::Data<AppState>) -> Option<HttpResponse> {
    Some(HttpResponse::Ok().json(&state.factions().iter().map(|(_, f)| f.clone()).collect::<Vec<Faction>>()))
}