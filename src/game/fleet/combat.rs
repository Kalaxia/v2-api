use std::collections::HashMap;
use crate::{
    game::{
        fleet::fleet::{Fleet, FleetID}
    }
};

pub fn engage(attacker: &mut Fleet, mut defenders: &mut HashMap<FleetID, Fleet>) -> bool {
    let nb_defense_ships = defenders.iter().map(|(_, f)| f.nb_ships).sum();

    if attacker.nb_ships > nb_defense_ships {
        attacker.nb_ships -= nb_defense_ships;
        return true;
    }
    attacker.nb_ships = 0;
    distribute_losses(&mut defenders, nb_defense_ships, attacker.nb_ships);
    false
}

fn distribute_losses(fleets: &mut HashMap<FleetID, Fleet>, total_ships: usize, losses: usize) {
    fleets.retain(|_, f| {
        let percent: f64 = f.nb_ships as f64 / total_ships as f64;
        f.nb_ships = (losses as f64 * percent).floor() as usize;
        f.nb_ships > 0
    });
}