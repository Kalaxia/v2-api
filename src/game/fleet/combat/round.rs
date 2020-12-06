use std::collections::HashMap;
use crate::{
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

pub async fn fight_round(mut battle: &mut Battle, number: u16, new_fleets: HashMap<FleetID, Fleet>) -> Option<Round> {
    let bid = battle.id;
    let new_round = move || {
        Round {
            battle: bid,
            fleet_actions: vec![],
            squadron_actions: vec![],
            number,
        }
    };

    let mut round = None;

    println!("PROCESSING ARRIVALS");
    // new fleets arrival
    for (_, fleet) in new_fleets.iter() {
        round
            .get_or_insert_with(new_round)
            .fleet_actions.push(FleetAction{
                battle: battle.id,
                fleet: fleet.id,
                kind: FleetActionKind::Join,
                round_number: number,
            });
    }

    // make each squadron fight
    for (fid, squadron) in battle.get_fighting_squadrons_by_initiative() {
        // a squadron may have no ennemy to attack, this is why we wrap its action into an Option
        attack(&mut battle, fid, &squadron, number)
        .map(|act| {
            round
                .get_or_insert_with(new_round)
                .squadron_actions.push(act);
        });
    }

    round
}

fn attack (battle: &mut Battle, fid: FactionID, attacker: &FleetSquadron, round_number: u16) -> Option<SquadronAction> {
    let (target_faction, target) = pick_target_squadron(&battle, fid, &attacker)?;
    let attack_coeff = attacker.formation.attack_coeff(target.formation);
    let (remaining_ships, loss) = fire(&attacker, &target, attack_coeff);

    for fs in battle.fleets.get_mut(&target_faction).unwrap().get_mut(&target.fleet).unwrap().squadrons.iter_mut() {
        if fs.id == target.id {
            fs.quantity = remaining_ships;
        }
    }

    Some(SquadronAction{
        battle: battle.id,
        squadron: attacker.id,
        kind: SquadronActionKind::Attack{ target: target.id, loss },
        round_number,
    })
}

/// This is an adaptation for multiple-fleet battles of Galadruin's battle idea (c.f. backlog
/// trello card).
///
/// In this version, overkill damages of one turn are not propagated to the next targeted
/// formation.
///
/// Also, when attacking, it is not fleet vs fleet but squadron vs squadron. Because of this, each
/// squadron of a fleet can attack a different fleet each turn.
fn pick_target_squadron(battle: &Battle, faction_id: FactionID, attacker: &FleetSquadron) -> Option<(FactionID, FleetSquadron)> {
    let mut potential_targets : Vec<(FactionID, &FleetSquadron)> = Vec::new();

    // c.f. game::fleet::formation::FleetFormation::attack_order()
    for target_formation in attacker.formation.attack_order() {
        potential_targets.extend(battle.fleets
            .iter()
            .filter(|(fid, _)| **fid != faction_id)
            .flat_map(|(fid, fleets)| fleets
                .iter()
                .flat_map(|(_, fleet)| &fleet.squadrons)
                .map(move |fs| (*fid, fs))
            )
            .filter(|(_, squadron)| squadron.formation == *target_formation && squadron.quantity > 0)
        );

        if !potential_targets.is_empty() { break }
    }

    if potential_targets.is_empty() { return None }

    let mut rng = thread_rng();

    potential_targets.choose(&mut rng).map(|(fid, fs)| (*fid, (*fs).clone()))
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
