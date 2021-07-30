use std::collections::{HashSet, HashMap};
use crate::{
    game::global::state,
    task,
    lib::{
        error::{ServerError, InternalError},
        time::Time,
        log::log,
        Result
    },
    game::{
        faction::FactionID,
        game::{game::GameID, server::GameServer},
        fleet::{
            combat::{
                conquest::Conquest,
                round::Round,
            },
            squadron::FleetSquadron,
            fleet::{Fleet, FleetID, get_fleet_player_ids},
        },
        system::system::{System, SystemID},
        player::{PlayerID, Player},
    },
    ws::protocol,
};
use actix::prelude::*;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Executor, Transaction, Postgres, Error, types::Json};
use sqlx_core::row::Row;
use rand::prelude::*;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct BattleID(pub Uuid);

#[derive(Serialize, Clone)]
pub struct Battle{
    pub id: BattleID,
    pub system: SystemID,
    pub attacker: FleetID,
    pub fleets: HashMap<FactionID, HashMap<FleetID, Fleet>>,
    pub rounds: Vec<Round>,
    pub defender_faction: Option<FactionID>,
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

impl<'a> FromRow<'a, PgRow<'a>> for Battle {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(Battle {
            id: row.try_get("id").map(BattleID)?,
            system: row.try_get("system_id").map(SystemID)?,
            attacker: row.try_get("attacker_id").map(FleetID)?,
            defender_faction: row.try_get("defender_faction_id").map(|id: i32| FactionID(id as u8)).ok(),
            fleets: (&*row.try_get::<Json<HashMap<FactionID, HashMap<FleetID, Fleet>>>, _>("fleets")?).clone(),
            rounds: (&*row.try_get::<Json<Vec<Round>>, _>("rounds")?).clone(),
            victor: row.try_get("victor_id").map(|id: i32| FactionID(id as u8)).ok(),
            begun_at: row.try_get("begun_at")?,
            ended_at: row.try_get("ended_at")?,
        })
    }
}

impl Battle {
    pub async fn find(bid: BattleID, db_pool: &PgPool) -> Result<Battle> {
        sqlx::query_as("SELECT * FROM fleet__combat__battles WHERE id = $1")
            .bind(Uuid::from(bid))
            .fetch_one(db_pool).await.map_err(ServerError::from)
    }

    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
    where
        E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO fleet__combat__battles(id, attacker_id, system_id, fleets, rounds, defender_faction_id, begun_at, ended_at) VALUES($1, $2, $3, $4, $5, $6, $7, $8)")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.attacker))
            .bind(Uuid::from(self.system))
            .bind(Json(&self.fleets))
            .bind(Json(&self.rounds))
            .bind(self.defender_faction.map(i32::from))
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

    pub async fn count_current_by_system(sid: &SystemID, db_pool: &PgPool) -> Result<i16> {
        sqlx::query_as("SELECT COUNT(*) FROM fleet__combat__battles WHERE system_id = $1 AND ended_at IS NULL")
            .bind(Uuid::from(*sid))
            .fetch_one(db_pool).await
            .map(|count: (i64,)| count.0 as i16)
            .map_err(ServerError::from)
    }

    pub async fn generate_reports<E>(&self, exec: &mut E) -> Result<()>
    where
        E: Executor<Database = Postgres> {
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
                        let initiative = (f64::from(squadron.category.to_data().combat_speed) * rng.gen_range(0.5, 1.5)).round() as i32;

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

    pub async fn engage(arriver: &Fleet, orbiting_fleets: &HashMap<FleetID, Fleet>, system: &System, defender_faction: Option<FactionID>, gid: GameID) -> Result<()> {
        let state = state();
        Conquest::stop(&system, gid).await?;
        
        let mut fleets = orbiting_fleets.clone();
        fleets.insert(arriver.id.clone(), arriver.clone());
    
        let battle = init_battle(arriver, system, fleets, defender_faction, &state.db_pool).await?;
    
        GameServer::ws_broadcast(gid, protocol::Message::new(protocol::Action::BattleStarted, &battle, None)).await?;
    
        let mut round = Round::new(battle.id, 1);
        state.games().get(&gid).unwrap().do_send(task!(round -> move |gs, ctx| {
            let gid = gs.id;
            ctx.wait(actix::fut::wrap_future(async move {
                round.execute(gid).await;
            }));
            Ok(())
        }));

        log(gelf::Level::Informational, "Battle started", "A new battle has started", vec![
            ("battle_id", battle.id.0.to_string()),
            ("system_id", system.id.0.to_string()),
            ("fleet_id", arriver.id.0.to_string()),
        ], &state.logger);

        Ok(())
    }

    // remaining fleets are grouped by faction ID. If there is less than two factions present, the fight is over
    pub fn is_over(&self) -> bool {
        2 > self.fleets.keys().len()
    }

    pub async fn end(&mut self, gid: GameID) -> Result<()> {
        let state = state();
        self.victor = Some(self.process_victor()?);
        self.ended_at = Some(Time::now());
        self.update(&mut &state.db_pool).await?;
        
        GameServer::ws_broadcast(gid, protocol::Message::new(
            protocol::Action::BattleEnded,
            self.clone(),
            None
        )).await?;

        if self.victor == self.defender_faction {
            return Ok(());
        }

        let fleet = Fleet::find(&self.attacker, &state.db_pool).await?;
        let system = System::find(self.system, &state.db_pool).await?;

        log(gelf::Level::Informational, "Battle ended", "A battle has finished", vec![
            ("battle_id", self.id.0.to_string()),
            ("victor_id", self.victor.unwrap().0.to_string()),
            ("system_id", self.system.0.to_string())
        ], &state.logger);

        Conquest::resume(&fleet, &system, self.victor, gid).await
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

pub async fn get_factions_fleets(fleets: HashMap<FleetID, Fleet>, db_pool: &PgPool) -> Result<HashMap<FactionID, HashMap<FleetID, Fleet>>> {
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

async fn init_battle(attacker: &Fleet, system: &System, fleets: HashMap<FleetID, Fleet>, defender_faction: Option<FactionID>, db_pool: &PgPool) -> Result<Battle> {
    let battle = Battle{
        id: BattleID(Uuid::new_v4()),
        attacker: attacker.id,
        system: system.id.clone(),
        fleets: get_factions_fleets(fleets, &db_pool).await?,
        rounds: vec![],
        defender_faction,
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

pub async fn update_fleets(battle: &Battle) -> Result<HashMap<FactionID, HashMap<FleetID, Fleet>>> {
    let state = state();
    let mut tx = state.db_pool.begin().await?;
    let mut remaining_fleets = HashMap::new();

    for (faction_id, fleets) in battle.fleets.iter() {
        let mut faction_remaining_fleets = HashMap::new();
        for (fleet_id, fleet) in fleets.iter() {
            let is_destroyed = update_fleet(fleet.clone(), &mut tx).await?;
            if is_destroyed {
                log(gelf::Level::Informational, "Fleet destroyed", "A fleet has been destroyed in combat", vec![
                    ("fleet_id", fleet.id.0.to_string()),
                    ("battle_id", battle.id.0.to_string()),
                ], &state.logger);
            } else {
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

async fn update_fleet(mut fleet: Fleet, tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<bool> {
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
