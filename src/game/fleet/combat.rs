use std::collections::HashMap;
use crate::{
    lib::Result,
    game::{
        fleet::fleet::{Fleet, FleetID},
        system::{System}
    }
};
use sqlx::{PgPool};

#[derive(serde::Serialize, Clone)]
pub struct CombatData {
    pub system: System,
    pub fleets: HashMap<FleetID, Fleet>,
}

pub async fn engage(attacker: &mut Fleet, mut defenders: &mut HashMap<FleetID, Fleet>, db_pool: &PgPool) -> Result<bool> {
    let nb_defense_ships = defenders.iter().map(|(_, f)| f.nb_ships).sum();

    if attacker.nb_ships > nb_defense_ships {
        attacker.nb_ships -= nb_defense_ships;
        return Ok(true);
    }
    distribute_losses(&mut defenders, nb_defense_ships, attacker.nb_ships, db_pool).await?;
    Fleet::remove(attacker.clone(), db_pool).await?;
    Ok(false)
}

async fn distribute_losses(fleets: &mut HashMap<FleetID, Fleet>, total_ships: usize, losses: usize, db_pool: &PgPool) -> Result<()> {
    for (_, f) in fleets.into_iter() {
        let percent: f64 = f.nb_ships as f64 / total_ships as f64;
        f.nb_ships -= (losses as f64 * percent).floor() as usize;
        if f.nb_ships > 0 {
            Fleet::update(f.clone(), db_pool).await?;
        } else {
            Fleet::remove(f.clone(), db_pool).await?;
        }
    }
    Ok(())
}