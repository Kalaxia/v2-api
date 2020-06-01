use actix_web::{get, web, HttpResponse};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use crate::{
    AppState
};

#[derive(Serialize, Deserialize, Clone)]
pub struct Faction{
    pub id: FactionID,
    name: String,
    color: (u8, u8, u8)
}

#[derive(Serialize, Deserialize, Copy, Clone, Hash, PartialEq, Eq, Debug)]
pub struct FactionID(pub u8);

pub fn generate_factions() -> HashMap<FactionID, Faction> {
    let mut factions = HashMap::new();
    factions.insert(FactionID(1), Faction{
        id: FactionID(1),
        name: String::from("Kalankar"),
        color: (255,200,0)
    });
    factions.insert(FactionID(2), Faction{
        id: FactionID(2),
        name: String::from("Valkar"),
        color: (0,0,255)
    });
    factions.insert(FactionID(3), Faction{
        id: FactionID(3),
        name: String::from("Adranite"),
        color: (255,0,0)
    });
    factions
}

#[get("/")]
pub async fn get_factions(state: web::Data<AppState>) -> Option<HttpResponse> {
    let mut factions = state
        .factions()
        .iter()
        .map(|(_, f)| f.clone())
        .collect::<Vec<Faction>>()
    ;
    factions.sort_by(|f1, f2| {
        let FactionID(fid1) = f1.id;
        let FactionID(fid2) = f2.id;
        fid1.cmp(&fid2)
    });
    Some(HttpResponse::Ok().json(factions))
}
