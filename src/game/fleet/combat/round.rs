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
        if let Some(act) = attack(&mut battle, fid, &squadron, number) {
            round
                .get_or_insert_with(new_round)
                .squadron_actions.push(act);
        }
    }

    round
}

fn attack (battle: &mut Battle, fid: FactionID, attacker: &FleetSquadron, round_number: u16) -> Option<SquadronAction> {
    let (target_faction, target) = pick_target_squadron(&battle, fid, &attacker)?;
    let (remaining_ships, loss) = fire(&attacker, &target);

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

fn fire(attacker: &FleetSquadron, defender: &FleetSquadron) -> (u16, u16) {
    let attacker_model = attacker.category.to_data();
    let attack_coeff = attacker.formation.attack_coeff(defender.formation);
    let defender_model = defender.category.to_data();

    let mut rng = thread_rng();
    let percent = rng.gen_range(attacker_model.precision as f64 / 2.0, attacker_model.precision as f64);

    let quantity = attacker.quantity as f64 * percent / 100.0;
    let damage = (quantity * attacker_model.damage as f64 * attack_coeff).ceil() as u16;
    let nb_casualties = (damage as f64 / defender_model.hit_points as f64).floor() as i32;
    let remaining_ships = defender.quantity as i32 - nb_casualties;

    if remaining_ships < 0 {
        return (0, defender.quantity);
    }
    (remaining_ships as u16, nb_casualties as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use crate::{
        lib::{
            time::Time,
        },
        game::{
            fleet::{
                fleet::{Fleet, FleetID},
                formation::{FleetFormation},
                squadron::{FleetSquadron, FleetSquadronID},
            },
            ship::model::ShipModelCategory,
            system::system::{SystemID},
            player::{PlayerID}
        }
    };

    #[test]
    fn test_pick_target_squadron() {
        let battle = get_battle_mock();
        let data =  vec![
            (1, 2, FleetFormation::Right),
            (1, 2, FleetFormation::Left),
            (2, 1, FleetFormation::Right),
            (2, 1, FleetFormation::Left),
        ];
        
        for (fid, tfid, formation) in data {
            let squadron = get_squadron_mock(ShipModelCategory::Corvette, formation, 5);
            let target = pick_target_squadron(&battle, FactionID(fid), &squadron);

            assert_eq!(true, target.is_some());

            let (target_faction, target_squadron) = target.unwrap();

            assert_eq!(FactionID(tfid), target_faction);
            assert_eq!(true, target_squadron.quantity > 0);
        }
    }

    #[test]
    fn test_fire() {
        let data = vec![
            (ShipModelCategory::Fighter, 10, ShipModelCategory::Fighter, 20, true),
            (ShipModelCategory::Corvette, 10, ShipModelCategory::Fighter, 20, true),
            (ShipModelCategory::Fighter, 1, ShipModelCategory::Cruiser, 20, false),
            (ShipModelCategory::Fighter, 100, ShipModelCategory::Cruiser, 20, true),
        ];

        for (cat, quantity, tcat, tquantity, has_casualties) in data {
            let attacker = get_squadron_mock(cat, FleetFormation::Right, quantity);
            let defender = get_squadron_mock(tcat, FleetFormation::Left, tquantity);

            let (remaining_ships, nb_casualties) = fire(&attacker, &defender);

            if has_casualties {
                assert_eq!(true, remaining_ships > 0);
                assert_eq!(true, remaining_ships < tquantity);
                assert_eq!(tquantity, nb_casualties + remaining_ships);
            } else {
                assert_eq!(true, remaining_ships == tquantity);
                assert_eq!(0, nb_casualties);
            }
        }
    }

    fn get_battle_mock() -> Battle {
        let mut faction_fleets = HashMap::new();
        let mut faction_1_fleets = HashMap::new();
        let mut faction_2_fleets = HashMap::new();

        faction_1_fleets.insert(FleetID(Uuid::new_v4()), get_fleet_mock());
        faction_1_fleets.insert(FleetID(Uuid::new_v4()), get_fleet_mock());
        faction_2_fleets.insert(FleetID(Uuid::new_v4()), get_fleet_mock());
        faction_2_fleets.insert(FleetID(Uuid::new_v4()), get_fleet_mock());
        faction_2_fleets.insert(FleetID(Uuid::new_v4()), get_fleet_mock());

        faction_fleets.insert(FactionID(1), faction_1_fleets);
        faction_fleets.insert(FactionID(2), faction_2_fleets);

        Battle{
            id: BattleID(Uuid::new_v4()),
            system: SystemID(Uuid::new_v4()),
            fleets: faction_fleets,
            rounds: vec![],
            victor: None,
            begun_at: Time::now(),
            ended_at: None,
        }
    }

    fn get_fleet_mock() -> Fleet {
        Fleet{
            id: FleetID(Uuid::new_v4()),
            player: PlayerID(Uuid::new_v4()),
            system: SystemID(Uuid::new_v4()),
            destination_system: None,
            destination_arrival_date: None,
            squadrons: vec![
                get_squadron_mock(ShipModelCategory::Fighter, FleetFormation::Left, 10),
                get_squadron_mock(ShipModelCategory::Fighter, FleetFormation::Rear, 20),
                get_squadron_mock(ShipModelCategory::Fighter, FleetFormation::Center, 10),
            ],
            is_destroyed: false,
        }
    }

    fn get_squadron_mock(category: ShipModelCategory, formation: FleetFormation, quantity: u16) -> FleetSquadron {
        FleetSquadron{
            id: FleetSquadronID(Uuid::new_v4()),
            fleet: FleetID(Uuid::new_v4()),
            formation,
            category,
            quantity,
        }
    }
}
