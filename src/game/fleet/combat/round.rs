use std::collections::HashMap;
use crate::{
    lib::{
        Result,
        error::{ServerError, InternalError},
        time::Time,
        auth::Claims
    },
    game::{
        faction::{FactionID},
        fleet::{
            formation::{FleetFormation},
            combat::{
                battle::{BattleID, Battle},
            },
            fleet::{FleetID},
            squadron::{FleetSquadronID, FleetSquadron},
        }
    }
};
use serde::{Serialize, Deserialize};
use rand::prelude::*;
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Executor, Error, Transaction, Postgres};
use uuid::Uuid;

#[derive(Serialize, Clone)]
pub struct Round {
    pub battle: BattleID,
    pub number: u16,
    pub fleet_actions: Vec<FleetAction>,
    pub squadron_actions: Vec<SquadronAction>,
}

#[derive(Serialize, Clone)]
pub struct ActionID(pub Uuid);

#[derive(Serialize, Clone)]
pub struct FleetAction {
    id: ActionID,
    fleet: FleetID,
    battle: BattleID,
    kind: FleetActionKind,
    round_number: u16
}

#[derive(Serialize, Clone, sqlx::Type)]
#[sqlx(rename = "VARCHAR")]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all(serialize = "snake_case", deserialize = "snake_case"))]
pub enum FleetActionKind {
    Join,
    Retreat,
    Surrender
}

#[derive(Serialize, Clone)]
pub struct SquadronAction {
    id: ActionID,
    squadron: FleetSquadronID,
    battle: BattleID,
    kind: SquadronActionKind,
    round_number: u16
}

#[derive(Serialize, Clone)]
pub enum SquadronActionKind {
    Attack { target: FleetSquadronID, loss: u16 }
}

impl From<ActionID> for Uuid {
    fn from(aid: ActionID) -> Self { aid.0 }
}

impl FleetAction{
    pub async fn create<E>(fa: FleetAction, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres>
    {
        sqlx::query("INSERT INTO fleet__combat__fleet_actions(id, fleet_id, battle_id, action, round_number) VALUES($1, $2, $3, $4, $5)")
            .bind(Uuid::from(fa.id))
            .bind(Uuid::from(fa.fleet))
            .bind(Uuid::from(fa.battle))
            .bind(fa.kind)
            .bind(i32::from(fa.round_number))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
}

impl SquadronAction{
    pub async fn create<E>(sa: SquadronAction, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres>
    {
        let (kind, fields, parameters): (String, String, Vec<String>) = match sa.kind {
            SquadronActionKind::Attack { target, loss } => ("attack".to_owned(), "target_id, loss".to_owned(), vec![Uuid::from(target).to_string(), loss.to_string()] )
        };
        let query_string = "INSERT INTO fleet__combat__squadron_actions(id, squadron_id, battle_id, action, ".to_owned() + &fields + ") VALUES($1, $2, $3, $4, $5)";
        let mut query = sqlx::query(&query_string)
            .bind(Uuid::from(sa.id))
            .bind(Uuid::from(sa.squadron))
            .bind(Uuid::from(sa.battle))
            .bind(kind)
            .bind(i32::from(sa.round_number));
        for parameter in parameters {
            query = query.bind(parameter);
        }
        query.execute(&mut *exec).await.map_err(ServerError::from)
    }
}

pub async fn fight_round(mut battle: &mut Battle, number: u16, db_pool: &PgPool) -> Result<Round> {
    let mut tx = db_pool.begin().await?;
    let mut round = Round{
        battle: battle.id.clone(),
        fleet_actions: vec![],
        squadron_actions: vec![],
        number,
    };

    // fleets actions
    for fleet in battle.get_joined_fleets(&db_pool).await? {
        let fa = FleetAction{
            id: ActionID(Uuid::new_v4()),
            battle: battle.id,
            fleet: fleet.id,
            kind: FleetActionKind::Join,
            round_number: number,
        };
        round.fleet_actions.push(fa.clone());
        FleetAction::create(fa, &mut tx).await?;
    }

    // squadrons actions
    for (fid, squadron) in battle.get_fighting_squadrons_by_initiative() {
        let sa = attack(&mut battle, fid, &squadron, number)?;
        round.squadron_actions.push(sa.clone());
        SquadronAction::create(sa, &mut tx).await?;
    }
    Ok(round)
}

fn attack (battle: &mut Battle, fid: FactionID, attacker: &FleetSquadron, round_number: u16) -> Result<SquadronAction> {
    let (target, attack_coeff) = pick_target_squadron(&battle, fid, &attacker)?;
    let (remaining_ships, loss) = fire(&attacker, &target, attack_coeff);

    battle.fleets.get_mut(&fid).unwrap()
        .get_mut(&target.fleet).unwrap()
        .squadrons.iter_mut().map(|mut fs| {
            if fs.id == target.id {
                fs.quantity = remaining_ships;
            }
            fs
        });

    Ok(SquadronAction{
        id: ActionID(Uuid::new_v4()),
        battle: battle.id,
        squadron: attacker.id,
        kind: SquadronActionKind::Attack{ target: target.id, loss },
        round_number,
    })
}

fn pick_target_squadron(battle: &Battle, faction_id: FactionID, attacker: &FleetSquadron) -> Result<(FleetSquadron, f64)> {
    let (opponent_squadrons, attack_coeff) = || -> Result<(Vec<FleetSquadron>, f64)> {
        for (target_formation, attack_coeff) in attacker.formation.get_attack_matrix() {
            let opponent_squadrons: Vec<FleetSquadron> = battle.fleets
                .iter()
                .filter(|(&fid, _)| fid != faction_id)
                .flat_map(|(_, fleets)| fleets)
                .flat_map(|(_, fleet)| fleet.squadrons.clone())
                .filter(|squadron| squadron.formation == target_formation && squadron.quantity > 0)
                .collect();

            if opponent_squadrons.len() > 0 {
                return Ok((opponent_squadrons, attack_coeff));
            }
        }
        return Err(InternalError::FleetEmpty)?;
    }()?;
    
    let mut rng = thread_rng();
    let idx = rng.gen_range(0, opponent_squadrons.len());

    Ok((opponent_squadrons[idx], attack_coeff))
}

fn fire(attacker: &FleetSquadron, defender: &FleetSquadron, attack_coeff: f64) -> (u16, u16) {
    let attacker_model = attacker.category.as_data();
    let defender_model = defender.category.as_data();

    let mut rng = thread_rng();
    let percent = rng.gen_range(attacker_model.precision as f64 / 2.0, attacker_model.precision as f64);

    let quantity = attacker.quantity as f64 * percent / 100.0;
    let damage = (quantity * attacker_model.damage as f64 * attack_coeff).ceil() as u16;
    let nb_casualties = (damage as f64 / defender_model.hit_points as f64).ceil() as i32;
    let remaining_ships = defender.quantity as i32 - nb_casualties;

    if remaining_ships < 0 {
        return (0, defender.quantity);
    }
    (remaining_ships as u16, nb_casualties as u16)
} 