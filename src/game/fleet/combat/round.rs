use std::collections::HashMap;
use crate::{
    lib::{
        Result,
        error::InternalError
    },
    game::{
        faction::{FactionID},
        fleet::{
            combat::{
                battle::{BattleID, Battle},
            },
            fleet::{FleetID, Fleet},
            squadron::{FleetSquadronID, FleetSquadron},
        }
    }
};
use serde::{Serialize};
use rand::prelude::*;

#[derive(Serialize, Clone)]
pub struct Round {
    pub battle: BattleID,
    pub number: u16,
    pub fleet_actions: Vec<FleetAction>,
    pub squadron_actions: Vec<SquadronAction>,
}

#[derive(Serialize, Clone, Copy)]
pub struct FleetAction {
    fleet: FleetID,
    battle: BattleID,
    kind: FleetActionKind,
    round_number: u16
}

#[derive(Serialize, Clone, Copy, sqlx::Type)]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum FleetActionKind {
    Join,
    Retreat,
    Surrender
}

#[derive(Serialize, Clone, Copy)]
pub struct SquadronAction {
    squadron: FleetSquadronID,
    battle: BattleID,
    kind: SquadronActionKind,
    round_number: u16
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum SquadronActionKind {
    Attack { target: FleetSquadronID, loss: u16 }
}

pub async fn fight_round(mut battle: &mut Battle, number: u16, new_fleets: HashMap<FleetID, Fleet>) -> Result<Round> {
    let mut round = Round{
        battle: battle.id.clone(),
        fleet_actions: vec![],
        squadron_actions: vec![],
        number,
    };

    // fleets actions
    for (_, fleet) in new_fleets.iter() {
        round.fleet_actions.push(FleetAction{
            battle: battle.id,
            fleet: fleet.id,
            kind: FleetActionKind::Join,
            round_number: number,
        });
    }

    // squadrons actions
    for (fid, squadron) in battle.get_fighting_squadrons_by_initiative() {
        round.squadron_actions.push(attack(&mut battle, fid, &squadron, number)?);
    }
    Ok(round)
}

fn attack (battle: &mut Battle, fid: FactionID, attacker: &FleetSquadron, round_number: u16) -> Result<SquadronAction> {
    let (target_faction, target, attack_coeff) = pick_target_squadron(&battle, fid, &attacker)?;
    let (remaining_ships, loss) = fire(&attacker, &target, attack_coeff);

    for fs in battle.fleets.get_mut(&target_faction).unwrap().get_mut(&target.fleet).unwrap().squadrons.iter_mut() {
        if fs.id == target.id {
            fs.quantity = remaining_ships;
        }
    }

    Ok(SquadronAction{
        battle: battle.id,
        squadron: attacker.id,
        kind: SquadronActionKind::Attack{ target: target.id, loss },
        round_number,
    })
}

fn pick_target_squadron(battle: &Battle, faction_id: FactionID, attacker: &FleetSquadron) -> Result<(FactionID, FleetSquadron, f64)> {
    let (opponent_squadrons, attack_coeff) = || -> Result<(Vec<(FactionID, FleetSquadron)>, f64)> {
        for (target_formation, attack_coeff) in attacker.formation.get_attack_matrix() {
            let opponent_squadrons: Vec<(FactionID, FleetSquadron)> = battle.fleets
                .iter()
                .filter(|(&fid, _)| fid != faction_id)
                .flat_map(|(fid, fleets)| fleets
                    .iter()
                    .flat_map(|(_, fleet)| fleet.squadrons.clone())
                    .map(|fs| (fid.clone(), fs))
                    .collect::<Vec<(FactionID, FleetSquadron)>>()
                )
                .filter(|(_, squadron)| squadron.formation == target_formation && squadron.quantity > 0)
                .collect();

            if opponent_squadrons.len() > 0 {
                return Ok((opponent_squadrons, attack_coeff));
            }
        }
        return Err(InternalError::FleetEmpty)?;
    }()?;
    
    let mut rng = thread_rng();
    let idx = rng.gen_range(0, opponent_squadrons.len());

    Ok((opponent_squadrons[idx].0, opponent_squadrons[idx].1, attack_coeff))
}

fn fire(attacker: &FleetSquadron, defender: &FleetSquadron, attack_coeff: f64) -> (u16, u16) {
    let attacker_model = attacker.category.to_data();
    let defender_model = defender.category.to_data();

    let mut rng = thread_rng();
    let percent = rng.gen_range(attacker_model.precision as f64 / 2.0, attacker_model.precision as f64);

    let quantity = attacker.quantity as f64 * percent / 100.0;
    let damage = (quantity * attacker_model.damage as f64 * attack_coeff).ceil() as u16;
    let nb_casualties = (damage as f64 / defender_model.hit_points as f64).ceil() as i32;
    let remaining_ships = defender.quantity as i32 - nb_casualties;

    if remaining_ships < 0 {
        return (0, defender.quantity);
    }
    (remaining_ships as u16, nb_casualties as u16)
} 