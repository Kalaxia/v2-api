use std::collections::{HashSet, HashMap};
use crate::{
    lib::{
        error::{ServerError, InternalError},
        time::Time,
        Result
    },
    game::{
        faction::FactionID,
        game::server::GameServer,
        fleet::{
            combat::{
                conquest::Conquest,
                round::{Round, fight_round},
            },
            squadron::FleetSquadron,
            fleet::{Fleet, FleetID, get_fleet_player_ids},
        },
        system::system::{System, SystemID},
        player::{PlayerID, Player},
    },
    ws::protocol,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, postgres::PgQueryResult, Executor, Transaction, Postgres, types::Json};
use rand::prelude::*;
use uuid::Uuid;
use std::time::Duration;
use std::thread;

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
    pub async fn insert<'a, E>(&self, exec: E) -> Result<PgQueryResult>
    where
        E: Executor<'a, Database = Postgres> {
        sqlx::query("INSERT INTO fleet__combat__battles(id, system_id, fleets, rounds, begun_at, ended_at) VALUES($1, $2, $3, $4, $5, $6)")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.system))
            .bind(Json(&self.fleets))
            .bind(Json(&self.rounds))
            .bind(self.begun_at)
            .bind(self.ended_at)
            .execute(exec).await.map_err(ServerError::from)
    }

    pub async fn update<'a, E>(&self, exec: E) -> Result<PgQueryResult>
    where
        E: Executor<'a, Database = Postgres> {
        sqlx::query("UPDATE fleet__combat__battles SET fleets = $2, rounds = $3, victor_id = $4, ended_at = $5 WHERE id = $1")
            .bind(Uuid::from(self.id))
            .bind(Json(&self.fleets))
            .bind(Json(&self.rounds))
            .bind(self.victor.map(i32::from))
            .bind(self.ended_at)
            .execute(exec).await.map_err(ServerError::from)
    }

    pub async fn get_joined_fleets(&self, db_pool: &PgPool) -> Result<Vec<Fleet>> {
        sqlx::query_as("SELECT * FROM fleet__fleets WHERE system_id = $1 AND destination_id IS NULL AND id != any($2)")
            .bind(Uuid::from(self.id))
            .bind(self.get_fleet_ids().into_iter().map(Uuid::from).collect::<Vec<Uuid>>())
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn count_current_by_system(sid: &SystemID, db_pool: &PgPool) -> Result<i16> {
        sqlx::query_as("SELECT COUNT(*) FROM fleet__combat__battles WHERE system_id = $1 AND ended_at IS NULL")
            .bind(Uuid::from(*sid))
            .fetch_one(db_pool).await
            .map(|count: (i64,)| count.0 as i16)
            .map_err(ServerError::from)
    }

    pub async fn generate_reports<'a, E>(&self, exec: &E) -> Result<()>
    where
        E: Executor<'a, Database = Postgres> + Copy {
        let mut players: HashSet<PlayerID> = HashSet::new();

        for fleets in self.fleets.values() {
            for fleet in fleets.values() {
                if !players.insert(fleet.player) {
                    continue;
                }
                let report = Report{
                    player: fleet.player,
                    battle: self.id,
                };
                report.insert(exec).await?;
            }
        }
        Ok(())
    }

    pub fn get_fighting_squadrons_by_initiative(&self) -> Vec<(FactionID, FleetSquadron)> {
        let mut squadrons: HashMap<i32, Vec<(FactionID, FleetSquadron)>> = HashMap::new();
        let mut rng = thread_rng();

        for (fid, fleets) in &self.fleets {
            for fleet in fleets.values() {
                for squadron in &fleet.squadrons {
                    if squadron.quantity > 0 {
                        let initiative = (f64::from(squadron.category.to_data().combat_speed) * rng.gen_range(0.5..1.5)).round() as i32;

                        squadrons.entry(initiative)
                            .or_default()
                            .push((*fid, squadron.clone()));
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
            .map(|t| *t.0)
            .collect()
    }

    fn process_victor(&self) -> Result<FactionID> {
        for (fid, fleets) in &self.fleets {
            for fleet in fleets.values() {
                if fleet.squadrons.iter().any(|s| s.quantity > 0) {
                    return Ok(*fid);
                }
            }
        }
        Err(InternalError::NotFound.into())
    }

    pub async fn prepare(fleet: &Fleet, fleets: &HashMap<FleetID, Fleet>, system: &System, defender_faction: Option<FactionID>, server: &GameServer) -> Result<()> {
        Conquest::stop(&system, &server).await?;
        
        let battle = Battle::engage(&system, &server, &fleet, &fleets).await?;

        server.ws_broadcast(&protocol::Message::new(
            protocol::Action::BattleEnded,
            battle.clone(),
            None
        ));

        if battle.victor == defender_faction {
            return Ok(());
        }

        Conquest::resume(&fleet, &system, battle.victor, &server).await
    }

    pub async fn engage(system: &System, server: &GameServer, arriver: &Fleet, orbiting_fleets: &HashMap<FleetID, Fleet>) -> Result<Battle> {
        let mut fleets = orbiting_fleets.clone();
        fleets.insert(arriver.id.clone(), arriver.clone());
    
        let mut battle = init_battle(system, fleets, &server.state.db_pool).await?;
    
        server.ws_broadcast(&protocol::Message::new(protocol::Action::BattleStarted, &battle, None));
    
        thread::sleep(Duration::new(3, 0));
    
        let mut round_number: u16 = 1;
        loop {
            let new_fleets = battle.get_joined_fleets(&server.state.db_pool).await?.iter().map(|f| (f.id.clone(), f.clone())).collect::<HashMap<FleetID, Fleet>>();
            for (fid, fleets) in get_factions_fleets(new_fleets.clone(), &server.state.db_pool).await? {
                battle.fleets.get_mut(&fid).unwrap().extend(fleets);
            }
    
            if let Some(round) = fight_round(&mut battle, round_number, new_fleets).await {
                battle.rounds.push(round);
                battle.update(&server.state.db_pool).await?;
                round_number += 1;
    
                thread::sleep(Duration::new(1, 0));
            } else {
                break;
            }
        }
        battle.victor = Some(battle.process_victor()?);
        battle.ended_at = Some(Time::now());
        battle.update(&server.state.db_pool).await?;
        battle.fleets = update_fleets(&battle, &server.state.db_pool).await?;
        Ok(battle)
    }
}

impl Report {
    pub async fn insert<'a, E>(&self, exec: E) -> Result<PgQueryResult>
    where
        E: Executor<'a, Database = Postgres> + Copy  {
        sqlx::query("INSERT INTO fleet__combat__reports(battle_id, player_id) VALUES($1, $2)")
            .bind(Uuid::from(self.battle))
            .bind(Uuid::from(self.player))
            .execute(exec).await.map_err(ServerError::from)
    }
}

async fn get_factions_fleets(fleets: HashMap<FleetID, Fleet>, db_pool: &PgPool) -> Result<HashMap<FactionID, HashMap<FleetID, Fleet>>> {
    let players: HashMap<PlayerID, Player> = Player::find_by_ids(get_fleet_player_ids(&fleets), &db_pool).await?
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

async fn update_fleets(battle: &Battle, db_pool: &PgPool) -> Result<HashMap<FactionID, HashMap<FleetID, Fleet>>> {
    let mut tx = db_pool.begin().await?;
    let mut remaining_fleets = HashMap::new();

    for (faction_id, fleets) in battle.fleets.iter() {
        let mut faction_remaining_fleets = HashMap::new();
        for (fleet_id, fleet) in fleets.iter() {
            let is_destroyed = update_fleet(fleet.clone(), &mut tx).await?;
            if !is_destroyed {
                faction_remaining_fleets.insert(*fleet_id, fleet.clone());
            }
        }
        if !faction_remaining_fleets.is_empty() {
            remaining_fleets.insert(*faction_id, faction_remaining_fleets);
        }
    }

    tx.commit().await?;

    Ok(remaining_fleets)
}

async fn update_fleet(mut fleet: Fleet, tx: &mut Transaction<'_, Postgres>) -> Result<bool> {
    for s in &fleet.squadrons {
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

    Ok(fleet.is_destroyed)
}
