use std::collections::{HashSet, HashMap};
use crate::{
    lib::{
        error::{ServerError, InternalError},
        time::Time,
        Result
    },
    game::{
        faction::FactionID,
        fleet::{
            combat::{
                round::{Round, fight_round},
            },
            squadron::FleetSquadron,
            fleet::{Fleet, FleetID},
        },
        system::system::{System, SystemID},
        player::{PlayerID, Player},
    }
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Executor, Error, Transaction, Postgres, types::Json};
use rand::prelude::*;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct BattleID(pub Uuid);

#[derive(Serialize, Clone)]
pub struct Battle{
    pub id: BattleID,
    pub system: SystemID,
    pub fleets: HashMap<FactionID, HashMap<FleetID, Fleet>>,
    pub rounds: Vec<Round>,
    pub victor: Option<FactionID>,
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
    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
    where
        E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO fleet__combat__battles(id, system_id, fleets, rounds, begun_at, ended_at) VALUES($1, $2, $3, $4, $5, $6)")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.system))
            .bind(Json(&self.fleets))
            .bind(Json(&self.rounds))
            .bind(self.begun_at)
            .bind(self.ended_at)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn update<E>(&self, exec: &mut E) -> Result<u64>
    where
        E: Executor<Database = Postgres> {
        sqlx::query("UPDATE fleet__combat__battles SET fleets = $2, rounds = $3, victor_id = $4, ended_at = $5 WHERE id = $1")
            .bind(Uuid::from(self.id))
            .bind(Json(&self.fleets))
            .bind(Json(&self.rounds))
            .bind(self.victor.map(i32::from))
            .bind(self.ended_at)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn get_joined_fleets(&self, db_pool: &PgPool) -> Result<Vec<Fleet>> {
        sqlx::query_as("SELECT * FROM fleet__fleets WHERE system_id = $1 AND destination_id IS NULL AND id != any($2)")
            .bind(Uuid::from(self.id))
            .bind(self.get_fleet_ids().into_iter().map(Uuid::from).collect::<Vec<Uuid>>())
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn generate_reports<E>(&self, exec: &mut E) -> Result<()>
    where
        E: Executor<Database = Postgres> {
        let mut players: HashSet<PlayerID> = HashSet::new();

        for (_, fleets) in self.fleets.iter() {
            for (_, fleet) in fleets {
                if !players.insert(fleet.player.clone()) {
                    continue;
                }
                let report = Report{
                    player: fleet.player.clone(),
                    battle: self.id.clone(),
                };
                report.insert(exec).await?;
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
                        let initiative = (f64::from(squadron.category.to_data().combat_speed) * rng.gen_range(0.5, 1.5)).round() as i32;

                        squadrons.entry(initiative)
                            .or_default()
                            .push((fid.clone(), squadron.clone()));
                    }
                }
            }
        }
        squadrons.values().flatten().cloned().collect()
    }

    pub fn get_fleet_ids(&self) -> Vec<FleetID> {
        self.fleets
            .iter()
            .flat_map(|t| t.1)
            .map(|t| t.0.clone())
            .collect()
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
}

impl Report {
    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
    where
        E: Executor<Database = Postgres>  {
        sqlx::query("INSERT INTO fleet__combat__reports(battle_id, player_id) VALUES($1, $2)")
            .bind(Uuid::from(self.battle))
            .bind(Uuid::from(self.player))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
}

async fn get_factions_fleets(fleets: HashMap<FleetID, Fleet>, db_pool: &PgPool) -> Result<HashMap<FactionID, HashMap<FleetID, Fleet>>> {
    let player_ids = fleets.iter().map(|(_, f)| f.player.clone()).collect();
    let players: HashMap<PlayerID, Player> = Player::find_by_ids(player_ids, &db_pool).await?
        .iter()
        .map(|p| (p.id, p.clone()))
        .collect();
    let mut faction_parties: HashMap<FactionID, HashMap<FleetID, Fleet>> = HashMap::new();

    for (fid, fleet) in fleets {
        let player = players.get(&fleet.player).unwrap();
        let faction = player.faction.unwrap();

        faction_parties.entry(faction)
            .or_default()
            .insert(fid, fleet);
    }
    Ok(faction_parties)
}

pub async fn engage(system: &System, arriver: Fleet, orbiting_fleets: HashMap<FleetID, Fleet>, mut db_pool: &PgPool) -> Result<Battle> {
    let mut fleets = orbiting_fleets.clone();
    fleets.insert(arriver.id.clone(), arriver.clone());

    let mut battle = init_battle(system, fleets, &db_pool).await?;
    let mut round_number: u16 = 1;

    loop {
        let new_fleets = battle.get_joined_fleets(&db_pool).await?.iter().map(|f| (f.id.clone(), f.clone())).collect::<HashMap<FleetID, Fleet>>();
        for (fid, fleets) in get_factions_fleets(new_fleets.clone(), &db_pool).await? {
            battle.fleets.get_mut(&fid).unwrap().extend(fleets);
        }

        if let Some(round) = fight_round(&mut battle, round_number, new_fleets).await {
            battle.rounds.push(round);
            battle.update(&mut db_pool).await?;
            round_number += 1;
        } else {
            break;
        }
    }
    update_fleets(&battle, db_pool).await?;
    battle.victor = Some(battle.process_victor()?);
    battle.ended_at = Some(Time::now());
    battle.update(&mut db_pool).await?;
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
    battle.insert(&mut tx).await?;
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
    for s in fleet.squadrons.iter() {
        if s.quantity > 0 {
            s.update(tx).await?;
        } else {
            s.remove(tx).await?;
        }
    }
    
    fleet.squadrons.retain(|s| s.quantity > 0);

    if fleet.squadrons.is_empty() {
        fleet.is_destroyed = true;
    }
    fleet.update(tx).await?;

    Ok(())
}
