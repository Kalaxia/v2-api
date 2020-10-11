use std::collections::HashMap;
use crate::{
    lib::{
        error::{ServerError, InternalError},
        time::Time,
        Result
    },
    game::{
        faction::{FactionID, Faction},
        fleet::{
            combat::{
                round::{Round, fight_round},
            },
            formation::{FleetFormation},
            squadron::{FleetSquadron, FleetSquadronID},
            fleet::{Fleet, FleetID},
        },
        system::system::{System, SystemID},
        player::{PlayerID, Player},
    }
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Executor, Error, Transaction, Postgres};
use rand::prelude::*;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct BattleID(pub Uuid);

#[derive(Serialize, Clone)]
pub struct Battle{
    pub id: BattleID,
    pub system: SystemID,
    pub fleets: HashMap<FactionID, HashMap<FleetID, Fleet>>,
    pub victor: Option<FactionID>,
    pub rounds: Vec<Round>,
    pub begun_at: Time,
    pub ended_at: Option<Time>,
}

#[derive(Serialize, Clone)]
pub struct Report {
    pub player: PlayerID,
    pub battle: BattleID,
}

impl From<BattleID> for Uuid {
    fn from(bid: BattleID) -> Self { bid.0 }
}

impl Battle {
    pub async fn create<E>(b: Battle, exec: &mut E) -> Result<u64>
    where
        E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO fleet__combat__battles(id, system_id, fleets, begun_at, ended_at) VALUES($1, $2, $3, $4, $5)")
            .bind(Uuid::from(b.id))
            .bind(Uuid::from(b.system))
            .bind(serde_json::to_string(&b.fleets).unwrap())
            .bind(b.begun_at)
            .bind(b.ended_at)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn update<E>(b: Battle, exec: &mut E) -> Result<u64>
    where
        E: Executor<Database = Postgres> {
        sqlx::query("UPDATE fleet__combat__battles SET fleets = $2 ended_at = $3 WHERE id = $1")
            .bind(Uuid::from(b.id))
            .bind(serde_json::to_string(&b.fleets).unwrap())
            .bind(b.ended_at)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn generate_reports<E>(&self, exec: &mut E) -> Result<()>
    where
        E: Executor<Database = Postgres> {
        let mut players: Vec<PlayerID> = vec![];

        for (_, fleets) in self.fleets.iter() {
            for (_, fleet) in fleets {
                if players.contains(&fleet.player) {
                    continue;
                }
                players.push(fleet.player.clone());
                Report::create(Report{
                    player: fleet.player.clone(),
                    battle: self.id.clone(),
                }, exec).await?;
            }
        }
        Ok(())
    }

    pub fn get_fighting_squadrons_by_initiative(&self) -> Vec<(FactionID, FleetSquadron)> {
        let mut squadrons: HashMap<i32, Vec<(FactionID, FleetSquadron)>> = HashMap::new();
        let mut rng = thread_rng();

        for (fid, fleets) in self.fleets.iter() {
            for (_, fleet) in fleets {
                for squadron in fleet.squadrons.iter() {
                    if squadron.quantity > 0 {
                        let initiative = (f64::from(squadron.category.as_data().combat_speed) * rng.gen_range(0.5, 1.5)).round() as i32;

                        if !squadrons.contains_key(&initiative) {
                            squadrons.insert(initiative, vec![]);
                        }
                        squadrons.get_mut(&initiative).unwrap().push((fid.clone(), squadron.clone()));
                    }
                }
            }
        }
        squadrons.values().flatten().cloned().collect()
    }

    pub fn get_fleet_ids(&self) -> Vec<FleetID> {
        let mut ids = vec![];

        for (_, fleets) in self.fleets.iter() {
            for (fid, _) in fleets {
                ids.push(fid.clone());
            }
        }
        ids
    }

    pub async fn get_joined_fleets(&self, db_pool: &PgPool) -> Result<Vec<Fleet>> {
        sqlx::query_as("SELECT * FROM fleet__fleets WHERE system_id = $1 AND destination_system_id IS NULL AND id != any($2))")
            .bind(Uuid::from(self.id))
            .bind(self.get_fleet_ids().into_iter().map(Uuid::from).collect::<Vec<Uuid>>())
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    fn process_victor(&self) -> Result<FactionID> {
        for (fid, fleets) in self.fleets.iter() {
            for (_, fleet) in fleets {
                if fleet.squadrons.iter().any(|s| s.quantity > 0) {
                    return Ok(fid.clone());
                }
            }
        }
        Err(InternalError::NotFound)?
    }

    fn is_fight_over(&self) -> bool {
        true
    }
}

impl Report {
    pub async fn create<E>(r: Report, exec: &mut E) -> Result<u64>
    where
        E: Executor<Database = Postgres>  {
        sqlx::query("INSERT INTO fleet__battle_reports(battle_id, player_id) VALUES($1, $2)")
            .bind(Uuid::from(r.battle))
            .bind(Uuid::from(r.player))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
}

async fn get_factions_fleets(fleets: HashMap<FleetID, Fleet>, db_pool: &PgPool) -> Result<HashMap<FactionID, HashMap<FleetID, Fleet>>> {
    let players: HashMap<PlayerID, Player> = Player::find_by_ids(fleets.iter().map(|(_, f)| f.player.clone()).collect(), &db_pool).await?
        .iter()
        .map(|p| (p.id, p.clone()))
        .collect();
    let mut faction_parties: HashMap<FactionID, HashMap<FleetID, Fleet>> = HashMap::new();

    for (fid, fleet) in fleets {
        let player = players.get(&fleet.player).unwrap();
        let faction = player.faction.unwrap();
        if !faction_parties.contains_key(&faction) {
            faction_parties.insert(faction, HashMap::new());
        }
        faction_parties.get_mut(&faction).unwrap().insert(fid, fleet);
    }
    Ok(faction_parties)
}

pub async fn engage(system: &System, arriver: Fleet, orbiting_fleets: HashMap<FleetID, Fleet>, mut db_pool: &PgPool) -> Result<Battle> {
    let mut fleets = orbiting_fleets.clone();
    fleets.insert(arriver.id.clone(), arriver.clone());

    let mut battle = init_battle(system, fleets, &db_pool).await?;
    let mut round_number: u16 = 1;

    loop {
        let round = fight_round(&mut battle, round_number, &db_pool).await;
        if round.is_err() {
            break;
        }
        battle.rounds.push(round.unwrap());
        round_number += 1;
    }
    update_fleets(&battle, db_pool).await?;
    battle.victor = Some(battle.process_victor()?);
    battle.ended_at = Some(Time::now());
    Battle::update(battle.clone(), &mut db_pool);
    Ok(battle)
}

async fn init_battle(system: &System, fleets: HashMap<FleetID, Fleet>, db_pool: &PgPool) -> Result<Battle> {
    let battle = Battle{
        id: BattleID(Uuid::new_v4()),
        system: system.id.clone(),
        fleets: get_factions_fleets(fleets, &db_pool).await?,
        rounds: vec![],
        victor: None,
        begun_at: Time::now(),
        ended_at: None,
    };
    // let mut players: HashMap<PlayerID, Player> = HashMap::new();
    let mut tx = db_pool.begin().await?;
    Battle::create(battle.clone(), &mut tx).await?;
    // for fleet in battle.fleets.iter() {
    //     if !players.contains_key(&fleet.player) {
    //         players.insert(fleet.player, Player::find(fleet.player, &db_pool).await?);
    //     }
    //     let player = players.get(&fleet.player).unwrap();
    //     battle.
    // }
    battle.generate_reports(&mut tx).await?;
    tx.commit().await?;

    Ok(battle)
}

async fn update_fleets(battle: &Battle, db_pool: &PgPool) -> Result<()> {
    let mut tx = db_pool.begin().await?;

    for (_, fleets) in battle.fleets.iter() {
        for (_, fleet) in fleets.iter() {
            update_fleet(fleet.clone(), &mut tx).await?;
        }
    }

    tx.commit().await?;

    Ok(())
}

async fn update_fleet(mut fleet: Fleet, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<()> {
    fleet.squadrons.retain(|s| s.quantity > 0);

    if fleet.squadrons.is_empty() {
        fleet.is_destroyed = true;
    }
    Fleet::update(fleet, tx).await?;

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
                formation::{FleetFormation},
                squadron::{FleetSquadron, FleetSquadronID},
            },
            ship::model::ShipModelCategory,
            system::system::{SystemID},
            player::{PlayerID}
        }
    };

    fn get_fleet_mock() -> Fleet {
        Fleet{
            id: FleetID(Uuid::new_v4()),
            player: PlayerID(Uuid::new_v4()),
            system: SystemID(Uuid::new_v4()),
            destination_system: None,
            destination_arrival_date: None,
            squadrons: vec![get_squadron_mock(ShipModelCategory::Fighter, 1)],
            is_destroyed: false,
        }
    }

    fn get_squadron_mock(category: ShipModelCategory, quantity: u16) -> FleetSquadron {
        FleetSquadron{
            id: FleetSquadronID(Uuid::new_v4()),
            fleet: FleetID(Uuid::new_v4()),
            formation: FleetFormation::Center,
            category,
            quantity,
        }
    }
}