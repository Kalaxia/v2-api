use std::collections::HashMap;
use crate::{
    lib::Result,
    game::{
        fleet::{
            ship::{get_ship_model, ShipGroup},
            fleet::{Fleet, FleetID},
        },
        system::{System}
    }
};
use sqlx::{PgPool, PgConnection, pool::PoolConnection, Transaction};
use rand::prelude::*;

#[derive(serde::Serialize, Clone)]
pub struct CombatData {
    pub system: System,
    pub fleets: HashMap<FleetID, Fleet>,
}

pub async fn engage(mut attacker: &mut Fleet, mut defenders: &mut HashMap<FleetID, Fleet>, db_pool: &PgPool) -> Result<bool> {
    loop {
        if !fight_round(attacker, defenders) || is_fight_over(&attacker, &defenders) {
            break;
        }
    }
    update_fleets(&mut attacker, &mut defenders, db_pool).await?;
    Ok(!attacker.ship_groups.is_empty())
}

fn is_fight_over(attacker: &Fleet, defenders: &HashMap<FleetID, Fleet>) -> bool {
    !attacker.can_fight() || !defenders.iter().any(|(_, f)| f.can_fight())
}

fn fight_round(mut attacker: &mut Fleet, defenders: &mut HashMap<FleetID, Fleet>) -> bool {
    let def_id = pick_target_fleet(defenders);
    if def_id.is_none() {
        return false;
    }

    let target_defender = defenders.get_mut(&def_id.unwrap()).unwrap();

    attack_fleet(&attacker, target_defender);

    for (_, defender) in defenders.iter_mut() {
        if attacker.can_fight() && defender.can_fight() {
            attack_fleet(&defender, &mut attacker);
        }
    }
    true
}

fn attack_fleet(attacker: &Fleet, defender: &mut Fleet) {
    let attacker_ship_group = pick_target_ship_group(&attacker);
    let defender_ship_group = pick_target_ship_group(&defender);

    defender.ship_groups[defender_ship_group].quantity = fire(
        &attacker.ship_groups[attacker_ship_group],
        &defender.ship_groups[defender_ship_group]
    );
}

fn pick_target_fleet(fleets: &HashMap<FleetID, Fleet>) -> Option<FleetID> {
    let fighting_fleets: HashMap::<FleetID, Fleet> = fleets
        .iter()
        .filter(|(_, f)| f.can_fight())
        .map(|(fid, fleet)| (fid.clone(), fleet.clone()))
        .collect();
    if fighting_fleets.is_empty() {
        return None;
    }

    let mut rng = thread_rng();
    let index = rng.gen_range(0, fighting_fleets.len());

    Some(fighting_fleets.keys().collect::<Vec<&FleetID>>()[index].clone())
}

fn pick_target_ship_group(fleet: &Fleet) -> usize {
    let fighting_groups: Vec<(usize, &ShipGroup)> = fleet.ship_groups
        .iter()
        .enumerate()
        .filter(|(_, sg)| sg.quantity > 0)
        .collect();
    
    let mut rng = thread_rng();
    let idx = rng.gen_range(0, fighting_groups.len());

    fighting_groups[idx].0
}

fn fire(attacker: &ShipGroup, defender: &ShipGroup) -> u16 {
    let attacker_model = get_ship_model(attacker.category.clone());
    let defender_model = get_ship_model(defender.category.clone());

    let mut rng = thread_rng();
    let percent = rng.gen_range(attacker_model.precision as f64 / 2.0, attacker_model.precision as f64);

    let quantity = (attacker.quantity as f64 * percent / 100.0).ceil() as u16;
    let damage = quantity * attacker_model.damage;
    let nb_casualties = (damage as f64 / defender_model.hit_points as f64).ceil() as i32;
    let remaining_ships = defender.quantity as i32 - nb_casualties;

    if remaining_ships < 0 {
        return 0;
    }
    remaining_ships as u16
}

async fn update_fleets(mut attacker: &mut Fleet, defenders: &mut HashMap<FleetID, Fleet>, db_pool: &PgPool) -> Result<()> {
    let mut tx = db_pool.begin().await?;

    update_fleet(&mut attacker, &mut tx).await?;

    for mut fleet in defenders.values_mut() {
        update_fleet(&mut fleet, &mut tx).await?;
    }

    tx.commit().await?;

    Ok(())
}

async fn update_fleet(fleet: &mut Fleet, mut tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<()> {
    for sg in fleet.ship_groups.iter() {
        if sg.quantity > 0 {
            ShipGroup::update(sg, tx).await?;
        } else {
            ShipGroup::remove(sg.id.clone(), &mut tx).await?;
        }
    }

    fleet.ship_groups.retain(|sg| sg.quantity > 0);

    if fleet.ship_groups.is_empty() {
        Fleet::remove(&fleet, &mut tx).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use crate::{
        game::{
            fleet::{
                fleet::{Fleet, FleetID},
                ship::{ShipGroup, ShipGroupID, ShipModelCategory},
            },
            system::{SystemID},
            player::{PlayerID}
        }
    };

    #[test]
    fn test_is_fight_over() {
        let mut attacker = get_fleet_mock();

        let mut defenders = HashMap::new();
        defenders.insert(FleetID(Uuid::new_v4()), get_fleet_mock());

        assert!(!is_fight_over(&attacker, &defenders));

        attacker.ship_groups[0].quantity = 0;

        assert!(is_fight_over(&attacker, &defenders));
    }

    #[test]
    fn test_pick_target_fleet() {
        let fighting_fleet_id = FleetID(Uuid::new_v4());

        let mut empty_fleet = get_fleet_mock();
        empty_fleet.ship_groups = vec![];

        let mut defenders = HashMap::new();
        defenders.insert(FleetID(Uuid::new_v4()), empty_fleet.clone());
        defenders.insert(fighting_fleet_id, get_fleet_mock());
        defenders.insert(FleetID(Uuid::new_v4()), empty_fleet.clone());

        assert!(Some(fighting_fleet_id.clone()) == pick_target_fleet(&defenders));

        defenders.remove(&fighting_fleet_id);

        assert!(pick_target_fleet(&defenders).is_none());
    }

    #[test]
    fn test_pick_target_ship_group() {
        let mut fleet = get_fleet_mock();
        fleet.ship_groups = vec![
            get_ship_group_mock(ShipModelCategory::Fighter, 0),
            get_ship_group_mock(ShipModelCategory::Corvette, 1),
            get_ship_group_mock(ShipModelCategory::Cruiser, 0)
        ];

        assert_eq!(pick_target_ship_group(&fleet), 1);
    }

    #[test]
    fn test_fire() {
        let attacker = get_ship_group_mock(ShipModelCategory::Corvette, 20);
        let defender = get_ship_group_mock(ShipModelCategory::Fighter, 100);

        let remaining_ships = fire(&attacker, &defender);

        assert!(remaining_ships > 20);
        assert!(remaining_ships < 80);
    }

    fn get_fleet_mock() -> Fleet {
        Fleet{
            id: FleetID(Uuid::new_v4()),
            player: PlayerID(Uuid::new_v4()),
            system: SystemID(Uuid::new_v4()),
            destination_system: None,
            destination_arrival_date: None,
            ship_groups: vec![get_ship_group_mock(ShipModelCategory::Fighter, 1)]
        }
    }

    fn get_ship_group_mock(category: ShipModelCategory, quantity: u16) -> ShipGroup {
        ShipGroup{
            id: ShipGroupID(Uuid::new_v4()),
            fleet: Some(FleetID(Uuid::new_v4())),
            system: None,
            category,
            quantity,
        }
    }
}