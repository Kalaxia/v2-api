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
        fight_round(attacker, defenders);

        if is_fight_over(&attacker, &defenders) { break; }
    }
    update_fleets(&mut attacker, &mut defenders, db_pool).await?;
    Ok(!attacker.ship_groups.is_empty())
}

fn is_fight_over(attacker: &Fleet, defenders: &HashMap<FleetID, Fleet>) -> bool {
    attacker.ship_groups.is_empty() || defenders.iter().any(|(_, f)| !f.ship_groups.is_empty())
}

fn fight_round(mut attacker: &mut Fleet, defenders: &mut HashMap<FleetID, Fleet>) {
    let def_id = pick_target_fleet(defenders).clone();
    let target_defender = defenders.get_mut(&def_id).unwrap();

    attack_fleet(&mut attacker, target_defender);

    for (_, mut defender) in defenders.iter_mut() {
        attack_fleet(&mut defender, &mut attacker);
    }
}

fn attack_fleet(attacker: &mut Fleet, defender: &mut Fleet) {
    let attacker_ship_group = pick_target_ship_group(&attacker);
    let defender_ship_group = pick_target_ship_group(&defender);

    defender.ship_groups[defender_ship_group].quantity = fire(
        &defender.ship_groups[defender_ship_group],
        &attacker.ship_groups[attacker_ship_group]
    );
}

fn pick_target_fleet(fleets: &HashMap<FleetID, Fleet>) -> &FleetID {
    let mut rng = thread_rng();
    let index = rng.gen_range(0, fleets.len());

    fleets.keys().collect::<Vec<&FleetID>>()[index]
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