use std::collections::HashMap;
use crate::{
    task,
    lib::{
        time::Time,
        error::InternalError,
        log::{log, Loggable},
        Result
    },
    game::{
        faction::{FactionID},
        fleet::{
            combat::{
                battle::{BattleID, Battle, get_factions_fleets, update_fleets},
            },
            fleet::{FleetID, Fleet},
            squadron::{FleetSquadronID, FleetSquadron},
        },
        player::ranking::PlayerRanking,
        game::server::{ GameServer, GameServerTask },
        ship::model::ShipModel,
    }
};
use futures::executor::block_on;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use rand::prelude::*;

#[derive(Deserialize, Serialize, Clone)]
pub struct Round {
    pub battle: BattleID,
    pub number: u16,
    pub fleet_actions: Vec<FleetAction>,
    pub squadron_actions: Vec<SquadronAction>,
}

#[derive(Deserialize, Serialize, Clone, Copy)]
pub struct FleetAction {
    fleet: FleetID,
    battle: BattleID,
    kind: FleetActionKind,
    round_number: u16
}

#[derive(Deserialize, Serialize, Clone, Copy, sqlx::Type)]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum FleetActionKind {
    Join,
    Retreat,
    Surrender
}

#[derive(Deserialize, Serialize, Clone, Copy)]
pub struct SquadronAction {
    squadron: FleetSquadronID,
    battle: BattleID,
    kind: SquadronActionKind,
    round_number: u16
}

#[derive(Deserialize, Serialize, Clone, Copy)]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum SquadronActionKind {
    Attack { target: FleetSquadronID, loss: u16 }
}

impl GameServerTask for Round {
    fn get_task_id(&self) -> String {
        format!("{}.{}", self.battle.0.to_string(), self.number.to_string())
    }

    fn get_task_end_time(&self) -> Time {
        let mut seconds = 1;
        if 1 == self.number {
            seconds = 3;
        }

        Time(Utc::now().checked_add_signed(Duration::seconds(seconds)).expect("Could not add round preparation time"))
    }
}

impl Round {
    pub const fn new(battle_id: BattleID, number: u16) -> Round
    {
        Round {
            battle: battle_id,
            fleet_actions: vec![],
            squadron_actions: vec![],
            number,
        }
    }

    pub async fn execute(&mut self, server: &GameServer) -> Result<()> {
        let mut battle = Battle::find(self.battle, &server.state.db_pool).await?;

        log(
            gelf::Level::Informational,
            "Battle round started",
            "A new round has been added to the battle",
            vec![
                ("battle_id", battle.id.0.to_string()),
                ("round_number", self.number.to_string()),
            ],
            &server.state.logger
        );
        
        let new_fleets = battle.get_joined_fleets(&server.state.db_pool).await?;

        for (faction_id, fleets) in get_factions_fleets(new_fleets.clone(), &server.state.db_pool).await? {
            if let Some(faction_fleets) = battle.fleets.get_mut(&faction_id) {
                faction_fleets.extend(fleets);
            } else {
                battle.fleets.insert(faction_id, fleets);
            }
        }

        self.fight(&mut battle, &new_fleets, &server).await?;
        battle.rounds.push(self.clone());
        battle.fleets = update_fleets(&battle, &server).await?;
        battle.update(&mut &server.state.db_pool).await?;

        if battle.is_over() {
            battle.end(server).await?;
        } else {
            let mut next_round = Round::new(battle.id, self.number + 1);
            server.state.games().get(&server.id).unwrap().do_send(task!(next_round -> move |gs| block_on(next_round.execute(gs))));
        }
        Ok(())
    }

    pub async fn fight(&mut self, mut battle: &mut Battle, new_fleets: &HashMap<FleetID, Fleet>, server: &GameServer) -> Result<()> {
        // new fleets arrival
        for fleet in new_fleets.values() {
            log(
                gelf::Level::Debug,
                "Fleet joined battle",
                &format!("Fleet {} has joined the fray", fleet.to_log_message()),
                vec![
                    ("battle_id", battle.id.0.to_string()),
                    ("system_id", battle.system.0.to_string()),
                ],
                &server.state.logger
            );
            self.fleet_actions.push(FleetAction{
                battle: battle.id,
                fleet: fleet.id,
                kind: FleetActionKind::Join,
                round_number: self.number,
            });
        }
    
        // make each squadron fight
        for (fid, squadron) in battle.get_fighting_squadrons_by_initiative(&new_fleets) {
            // a squadron may have no ennemy to attack, this is why we wrap its action into an Option
            if let Some(act) = attack(&mut battle, fid, &squadron, self.number, &new_fleets, &server).await? {
                self.squadron_actions.push(act);
            }
        }
        Ok(())
    }
}

async fn attack(
    battle: &mut Battle,
    fid: FactionID,
    attacker: &FleetSquadron,
    round_number: u16,
    excluded_fleets: &HashMap<FleetID, Fleet>,
    server: &GameServer
) -> Result<Option<SquadronAction>> {
    let (target_faction, target) = match pick_target_squadron(&battle, fid, &attacker, &excluded_fleets) {
        Some(target_squadron) => target_squadron,
        None => return Ok(None)
    };
    let (remaining_ships, loss, damage) = fire(&attacker, &target);

    log(
        gelf::Level::Debug,
        "Squadron attack",
        &format!(
            "Squadron {} of fleet {} containings {} ships has destroyed {}/{} ships from squadron {} of fleet {}",
            attacker.to_log_message(),
            attacker.fleet.to_string(),
            attacker.quantity.to_string(),
            loss.to_string(),
            target.quantity.to_string(),
            target.to_log_message(),
            target.fleet.to_string()
        ),
        vec![],
        &server.state.logger
    );

    let mut tx = server.state.db_pool.begin().await?;
    let loss_strength = target.category.to_data().strength * loss;

    if let Some(attacker_faction_fleets) = battle.fleets.get(&fid) {
        if let Some(attacker_fleet) = attacker_faction_fleets.get(&attacker.fleet) {
            if 0 < loss {
                PlayerRanking::add_destroyed_ships(attacker_fleet.player, target.category, loss as i32, loss_strength, &mut tx).await?;
            }
        } else {
            log(
                gelf::Level::Warning,
                "Attacker fleet not found",
                &format!(
                    "Attacker fleet {} of faction {} was not found in the battle data",
                    attacker.fleet.to_string(),
                    fid.0.to_string()
                ),
                vec![
                    ("attacker_fleet", attacker.fleet.to_string()),
                    ("attacker_faction", fid.0.to_string())
                ],
                &server.state.logger
            );
            return Ok(None);
        }
    } else {
        log(
            gelf::Level::Warning,
            "Attacker faction not found",
            &format!(
                "Attacker faction {} was not found in the battle data",
                fid.0.to_string()
            ),
            vec![
                ("attacker_faction", fid.0.to_string())
            ],
            &server.state.logger
        );
        return Ok(None);
    }
    if let Some(target_faction_fleets) = battle.fleets.get_mut(&target_faction) {
        if let Some(target_fleet) = target_faction_fleets.get_mut(&target.fleet) {
            if 0 < loss {
                PlayerRanking::add_lost_ships(target_fleet.player, target.category, loss as i32, loss_strength, &mut tx).await?;
            
                target_fleet.squadrons.iter_mut().filter(|fs| fs.id == target.id).for_each(|fs| {
                    fs.quantity = remaining_ships;
                });
            } else {
                target_fleet.squadrons.iter_mut().filter(|fs| fs.id == target.id).for_each(|fs| {
                    fs.damage = damage;
                });
            }
        } else {
            log(
                gelf::Level::Warning,
                "Target fleet not found",
                &format!(
                    "Target fleet {} of faction {} was not found in the battle data",
                    target.fleet.to_string(),
                    target_faction.0.to_string()
                ),
                vec![
                    ("target_fleet", target.fleet.to_string()),
                    ("target_faction", target_faction.0.to_string())
                ],
                &server.state.logger
            );
            return Ok(None);
        }
    } else {
        log(
            gelf::Level::Warning,
            "Target faction not found",
            &format!(
                "Target faction {} was not found in the battle data",
                target_faction.0.to_string()
            ),
            vec![
                ("target_faction", target_faction.0.to_string())
            ],
            &server.state.logger
        );
        return Ok(None);
    }

    tx.commit().await?;

    Ok(Some(SquadronAction{
        battle: battle.id,
        squadron: attacker.id,
        kind: SquadronActionKind::Attack{ target: target.id, loss },
        round_number,
    }))
}

/// This is an adaptation for multiple-fleet battles of Galadruin's battle idea (c.f. backlog
/// trello card).
///
/// In this version, overkill damages of one turn are not propagated to the next targeted
/// formation.
///
/// Also, when attacking, it is not fleet vs fleet but squadron vs squadron. Because of this, each
/// squadron of a fleet can attack a different fleet each turn.
fn pick_target_squadron(battle: &Battle, faction_id: FactionID, attacker: &FleetSquadron, excluded_fleets: &HashMap<FleetID, Fleet>) -> Option<(FactionID, FleetSquadron)> {
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
            .filter(|(_, squadron)| !excluded_fleets.contains_key(&squadron.fleet) && squadron.formation == *target_formation && squadron.quantity > 0)
        );

        if !potential_targets.is_empty() { break }
    }

    if potential_targets.is_empty() { return None }

    let mut rng = thread_rng();

    potential_targets.choose(&mut rng).map(|(fid, fs)| (*fid, (*fs).clone()))
}

fn fire(attacker: &FleetSquadron, defender: &FleetSquadron) -> (u16, u16, u16) {
    let attacker_model = attacker.category.to_data();
    let attack_coeff = attacker.formation.attack_coeff(defender.formation);
    let defender_model = defender.category.to_data();

    let mut rng = thread_rng();
    let percent = rng.gen_range(attacker_model.precision as f64 / 2.0, attacker_model.precision as f64);

    let quantity = attacker.quantity as f64 * percent / 100.0;
    let damage = (quantity * attacker_model.damage as f64 * attack_coeff).ceil() as u16 + defender.damage;

    let (nb_casualties, remaining_damage) = calculate_casualties(damage as f64, defender_model.hit_points as f64);
    let remaining_ships = defender.quantity as i32 - nb_casualties;
    let final_damage = {
        if 0 < nb_casualties {
            remaining_damage as u16
        } else {
            defender.damage + remaining_damage as u16
        }
    };
    
    if remaining_ships < 0 {
        return (0, defender.quantity, 0);
    }
    (remaining_ships as u16, nb_casualties as u16, final_damage as u16)
}

fn calculate_casualties(damage: f64, hit_points: f64) -> (i32, i32) {
    (
        (damage / hit_points).floor() as i32,
        (damage % hit_points) as i32
    )
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
            player::player::{PlayerID}
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
        let mut excluded_fleets = HashMap::new();
        excluded_fleets.insert(FleetID(Uuid::new_v4()), get_fleet_mock());
        
        for (fid, tfid, formation) in data {
            let squadron = get_squadron_mock(ShipModelCategory::Corvette, formation, 5, 0);
            let target = pick_target_squadron(&battle, FactionID(fid), &squadron, &excluded_fleets);

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
            let attacker = get_squadron_mock(cat, FleetFormation::Right, quantity, 0);
            let defender = get_squadron_mock(tcat, FleetFormation::Left, tquantity, 0);

            let (remaining_ships, nb_casualties, _damage) = fire(&attacker, &defender);

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

    #[test]
    fn test_fire_damage() {
        let attacker = get_squadron_mock(ShipModelCategory::Corvette, FleetFormation::Center, 1, 0);
        let defender = get_squadron_mock(ShipModelCategory::Corvette, FleetFormation::Center, 1, 0);

        let (remaining_ships, nb_casualties, damage) = fire(&attacker, &defender);

        assert_eq!(0, nb_casualties);
        assert_eq!(1, remaining_ships);
        assert_eq!(true, damage > 0);
    }

    #[test]
    fn test_fire_with_damaged_ships() {
        let attacker = get_squadron_mock(ShipModelCategory::Corvette, FleetFormation::Center, 1, 0);
        let defender = get_squadron_mock(ShipModelCategory::Corvette, FleetFormation::Center, 1, 55);
        
        let (remaining_ships, nb_casualties, damage) = fire(&attacker, &defender);

        assert_eq!(1, nb_casualties);
        assert_eq!(0, remaining_ships);
        assert_eq!(true, damage < 55);
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
            attacker: FleetID(Uuid::new_v4()),
            defender_faction: None,
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
                get_squadron_mock(ShipModelCategory::Fighter, FleetFormation::Left, 10, 0),
                get_squadron_mock(ShipModelCategory::Fighter, FleetFormation::Rear, 20, 0),
                get_squadron_mock(ShipModelCategory::Fighter, FleetFormation::Center, 10, 0),
            ],
            is_destroyed: false,
        }
    }

    fn get_squadron_mock(category: ShipModelCategory, formation: FleetFormation, quantity: u16, damage: u16) -> FleetSquadron {
        FleetSquadron{
            id: FleetSquadronID(Uuid::new_v4()),
            fleet: FleetID(Uuid::new_v4()),
            formation,
            category,
            quantity,
            damage,
        }
    }
}
