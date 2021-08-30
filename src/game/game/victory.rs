use serde::{Serialize};
use crate::{
    lib::{
        log::log,
        Result
    },
    game::{
        faction::{FactionID, GameFaction},
        game::{
            game::{Game, VICTORY_POINTS_PER_MINUTE},
            server::GameServer,
        },
        player::{Player, PlayerID},
        system::{
            system::{System, SystemDominion}
        },
    },
    ws::protocol,
};
use std::collections::HashMap;

#[derive(Serialize, Clone)]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum VictoryKind {
    Forfeit,
    Supremacy,
    Territorial
}

#[derive(Serialize, Clone)]
struct VictoryData {
    kind: VictoryKind,
    victorious_faction: FactionID,
    scores: Vec<GameFaction>
}

pub async fn distribute_victory_points(gs: &mut GameServer) -> Result<()> {
    let victory_systems = System::find_possessed_victory_systems(gs.id.clone(), &gs.state.db_pool).await?;
    let game = Game::find(gs.id.clone(), &gs.state.db_pool).await?;
    let mut factions = GameFaction::find_all(gs.id.clone(), &gs.state.db_pool).await?
        .into_iter()
        .map(|gf| (gf.faction.clone(), gf))
        .collect::<HashMap<FactionID, GameFaction>>();
    let mut players = Player::find_by_ids(victory_systems.clone().into_iter().map(|s| s.player.clone().unwrap()).collect(), &gs.state.db_pool).await?
        .into_iter()
        .map(|p| (p.id.clone(), p))
        .collect::<HashMap<PlayerID, Player>>();

    for system in victory_systems.iter() {
        factions.get_mut(
            &players.get_mut(&system.player.unwrap())
                .unwrap()
                .faction
                .unwrap()
        ).unwrap().victory_points += VICTORY_POINTS_PER_MINUTE;
    }

    let mut victorious_faction: Option<&GameFaction> = None;
    let mut tx = gs.state.db_pool.begin().await?;
    for f in factions.values() {
        GameFaction::update(f, &mut tx).await?;
        if f.victory_points >= game.victory_points {
            victorious_faction = Some(f);
        }
    }
    tx.commit().await?;

    gs.ws_broadcast(&protocol::Message::new(
        protocol::Action::FactionPointsUpdated,
        factions.clone(),
        None
    )).await?;

    if let Some(f) = victorious_faction {
        log(
            gelf::Level::Informational,
            &format!("Faction {} won by territory", f.faction.0.to_string()),
            &format!("Faction {} won by territory", f.faction.0.to_string()),
            vec![],
            &gs.state.logger
        );

        process_victory(VictoryKind::Territorial, &gs, f, factions.values().cloned().collect::<Vec<GameFaction>>()).await?;
    }

    Ok(())
}

async fn process_victory(kind: VictoryKind, gs: &GameServer, victorious_faction: &GameFaction, factions: Vec<GameFaction>) -> Result<()> {
    gs.ws_broadcast(&protocol::Message::new(
        protocol::Action::Victory,
        VictoryData{
            kind,
            victorious_faction: victorious_faction.faction,
            scores: factions,
        },
        None,
    )).await?;

    let game = Game::find(gs.id, &gs.state.db_pool).await?;
    gs.state.clear_game(&game).await?;
    Ok(())
}

pub async fn check_supremacy_victory(gs: &GameServer) -> Result<()> {
    let faction_systems_count = System::count_by_faction(gs.id, &gs.state.db_pool).await?;

    let remaining_factions: Vec<&SystemDominion> = faction_systems_count.iter().filter(|count| 0 < count.nb_systems).collect();

    if 1 == remaining_factions.len() {
        process_supremacy_victory(gs, remaining_factions[0].faction_id).await?;
    }
    
    Ok(())
}

async fn process_supremacy_victory(gs: &GameServer, faction_id: FactionID) -> Result<()> {
    let victorious_faction = GameFaction::find(gs.id, faction_id, &gs.state.db_pool).await?;
    let factions = GameFaction::find_all(gs.id, &gs.state.db_pool).await?;

    log(
        gelf::Level::Informational,
        &format!("Faction {} won by supremacy", victorious_faction.faction.0.to_string()),
        &format!("Faction {} won by supremacy", victorious_faction.faction.0.to_string()),
        vec![],
        &gs.state.logger
    );

    process_victory(VictoryKind::Supremacy, gs, &victorious_faction, factions).await?;

    Ok(())
}

pub async fn check_forfeit_victory(gs: &GameServer) -> Result<()> {
    if 1 == GameFaction::count_remaining(&gs.id, &gs.state.db_pool).await? {
        process_forfeit_victory(gs).await?;
    }

    Ok(())
}

async fn process_forfeit_victory(gs: &GameServer) -> Result<()> {
    let victorious_faction = GameFaction::find_remaining(gs.id, &gs.state.db_pool).await?;
    let factions = GameFaction::find_all(gs.id, &gs.state.db_pool).await?;

    log(
        gelf::Level::Informational,
        &format!("Faction {} won by forfeit", victorious_faction.faction.0.to_string()),
        &format!("Faction {} won by forfeit", victorious_faction.faction.0.to_string()),
        vec![],
        &gs.state.logger
    );
    
    process_victory(VictoryKind::Forfeit, gs, &victorious_faction, factions).await?;

    Ok(())
}