use uuid::Uuid;
use serde::{Serialize, Deserialize};
use std::{ops::IndexMut, collections::HashMap};
use crate::{
    lib::{Result, error::{ServerError, InternalError}},
    game::{
        faction::{FactionID},
        fleet::combat,
        fleet::fleet::{FleetID, Fleet},
        game::{GameID, MAP_SIZE},
        player::{PlayerID, Player}
    },
    ws::protocol
};
use petgraph::Graph;
use galaxy_rs::{GalaxyBuilder, Point, DataPoint};
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Error, Transaction};
use sqlx_core::row::Row;
use rand::prelude::*;

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct SystemID(pub Uuid);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct System {
    pub id: SystemID,
    pub game: GameID,
    pub player: Option<PlayerID>,
    pub coordinates: Coordinates,
    pub unreachable: bool
}

#[derive(Serialize, Clone)]
pub struct SystemDominion {
    pub faction_id: FactionID,
    pub nb_systems: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Coordinates {
    pub x: f64,
    pub y: f64
}

#[derive(Serialize, Clone)]
pub struct ConquestData {
    pub system: System,
    pub fleet: Fleet,
}

#[derive(Clone)]
pub enum FleetArrivalOutcome {
    Conquerred{
        system: System,
        fleet: Fleet,
    },
    Defended{
        system: System,
        fleets: HashMap<FleetID, Fleet>,
    },
    Arrived{
        fleet: Fleet,
    }
}

impl Coordinates {
    pub fn polar(r:f64, theta:f64) -> Coordinates {
        Coordinates {
            x: r * theta.cos(),
            y: r * theta.sin(),
        }
    }

    pub fn dot(&self, rhs:&Coordinates) -> f64 {
        self.x * rhs.x + self.y * rhs.y
    }
}

impl From<SystemID> for Uuid {
    fn from(sid: SystemID) -> Self { sid.0 }
}

impl<'a> FromRow<'a, PgRow<'a>> for Coordinates {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(Coordinates{
            x: row.try_get("coord_x")?,
            y: row.try_get("coord_y")?,
        })
    }
}

impl<'a> FromRow<'a, PgRow<'a>> for System {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        let id : Uuid = row.try_get("id")?;
        let game_id : Uuid = row.try_get("game_id")?;
        let player_id = match row.try_get("player_id") {
            Ok(pid) => Some(PlayerID(pid)),
            Err(_) => None
        };

        Ok(System {
            id: SystemID(id),
            game: GameID(game_id),
            player: player_id,
            coordinates: Coordinates{
                x: row.try_get("coord_x")?,
                y: row.try_get("coord_y")?,
            },
            unreachable: row.try_get("is_unreachable")?,
        })
    }
}

impl<'a> FromRow<'a, PgRow<'a>> for SystemDominion {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        let id : i32 = row.try_get("faction_id")?;

        Ok(SystemDominion {
            faction_id: FactionID(id as u8),
            nb_systems: row.try_get::<i64, _>("nb_systems")? as u32,
        })
    }
}

impl Coordinates {
    pub async fn get_max(gid: &GameID, db_pool: &PgPool) -> Result<Coordinates> {
        sqlx::query_as("SELECT MAX(coord_x) as coord_x, MAX(coord_y) as coord_y FROM map__systems WHERE game_id = $1")
            .bind(Uuid::from(gid.clone()))
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(InternalError::SystemUnknown))
    }
}

impl System {
    pub async fn resolve_fleet_arrival(&mut self, mut fleet: Fleet, player: &Player, system_owner: Option<Player>, db_pool: &PgPool) -> Result<FleetArrivalOutcome> {
        match system_owner {
            Some(system_owner) => {
                // Both players have the same faction, the arrived fleet just parks here
                if system_owner.faction == player.faction {
                    fleet.change_system(self);
                    Fleet::update(fleet.clone(), db_pool).await?;
                    return Ok(FleetArrivalOutcome::Arrived{ fleet });
                }
                // Conquest of the system by the arrived fleet
                let mut fleets: HashMap<FleetID, Fleet> = Fleet::find_stationed_by_system(&self.id, db_pool).await
                    .into_iter()
                    .map(|f| (f.id.clone(), f))
                    .collect();
                if fleets.is_empty() || combat::engage(&mut fleet, &mut fleets, db_pool).await? == true {
                    return self.conquer(fleet, db_pool).await;
                }
                fleets.insert(fleet.id.clone(), fleet.clone());
                Ok(FleetArrivalOutcome::Defended{ fleets, system: self.clone() })
            },
            None => self.conquer(fleet, db_pool).await
        }
    }

    pub async fn conquer(&mut self, mut fleet: Fleet, db_pool: &PgPool) -> Result<FleetArrivalOutcome> {
        Fleet::remove_defenders(&self.id, db_pool).await?;
        fleet.change_system(self);
        Fleet::update(fleet.clone(), db_pool).await?;
        self.player = Some(fleet.player.clone());
        System::update(self.clone(), db_pool).await?;
        Ok(FleetArrivalOutcome::Conquerred{
            system: self.clone(),
            fleet: fleet,
        })
    }

    pub async fn find(sid: SystemID, db_pool: &PgPool) -> Result<System> {
        sqlx::query_as("SELECT * FROM map__systems WHERE id = $1")
            .bind(Uuid::from(sid))
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(InternalError::SystemUnknown))
    }

    pub async fn find_unoccupied(gid: GameID, x: f64, y: f64, db_pool: &PgPool) -> Result<System> {
        sqlx::query_as("SELECT * FROM (
                (SELECT * FROM map__systems WHERE game_id = $1 AND player_id IS NULL AND coord_x >= $2 AND coord_y >= $3 ORDER BY coord_x, coord_y LIMIT 1)
                    UNION ALL
                (SELECT * FROM map__systems WHERE game_id = $1 AND player_id IS NULL AND coord_x < $2 AND coord_y < $3 ORDER BY coord_x DESC, coord_y DESC LIMIT 1)
            ) as system ORDER BY abs($2 - coord_x), abs($3 - coord_y) LIMIT 1")
            .bind(Uuid::from(gid))
            .bind(x)
            .bind(y)
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(InternalError::SystemUnknown))
    }

    pub async fn find_possessed(gid: GameID, db_pool: &PgPool) -> Vec<System> {
        sqlx::query_as("SELECT * FROM map__systems WHERE game_id = $1 AND player_id IS NOT NULL")
            .bind(Uuid::from(gid))
            .fetch_all(db_pool).await.expect("Could not retrieve possessed systems")
    }

    pub async fn find_all(gid: &GameID, db_pool: &PgPool) -> Vec<System> {
        sqlx::query_as("SELECT * FROM map__systems WHERE game_id = $1")
            .bind(Uuid::from(gid.clone()))
            .fetch_all(db_pool).await.expect("Could not retrieve systems")
    }

    pub async fn count_by_faction(gid: GameID, db_pool: &PgPool) -> Vec<SystemDominion> {
        sqlx::query_as(
            "SELECT f.id as faction_id, COUNT(s.*) as nb_systems FROM map__systems s
            INNER JOIN player__players p ON s.player_id = p.id
            INNER JOIN faction__factions f ON p.faction_id = f.id
            WHERE s.game_id = $1
            GROUP BY f.id")
        .bind(Uuid::from(gid))
        .fetch_all(db_pool).await.expect("Could not retrieve systems per faction")
    }

    pub async fn count(gid: GameID, db_pool: &PgPool) -> u32 {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM map__systems WHERE game_id = $1")
            .bind(Uuid::from(gid))
            .fetch_one(db_pool).await.expect("Could not retrieve systems");
        count.0 as u32
    }

    pub async fn create(s: System,  tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("INSERT INTO map__systems (id, game_id, player_id, coord_x, coord_y, is_unreachable) VALUES($1, $2, $3, $4, $5, $6)")
            .bind(Uuid::from(s.id))
            .bind(Uuid::from(s.game))
            .bind(s.player.map(Uuid::from))
            .bind(s.coordinates.x)
            .bind(s.coordinates.y)
            .bind(s.unreachable)
            .execute(tx).await.map_err(ServerError::from)
    }

    pub async fn update(s: System, db_pool: &PgPool) -> Result<u64> {
        sqlx::query("UPDATE map__systems SET player_id = $1, is_unreachable = $2 WHERE id = $3")
            .bind(s.player.map(Uuid::from))
            .bind(s.unreachable)
            .bind(Uuid::from(s.id))
            .execute(db_pool).await.map_err(ServerError::from)
    }

    pub async fn insert_all<I>(systems:I, pool:&PgPool) -> Result<u64>
        where I : IntoIterator<Item=System>
    {
        let mut tx = pool.begin().await?;
        let mut nb_inserted = 0;
        for sys in systems {
            nb_inserted += 1;
            System::create(sys.clone(), &mut tx).await?;
        }

        tx.commit().await?;
        Ok(nb_inserted)
    }
}

impl From<FleetArrivalOutcome> for protocol::Message {
    fn from(outcome: FleetArrivalOutcome) -> Self {
        match outcome {
            FleetArrivalOutcome::Conquerred { system, fleet } => protocol::Message::new(
                protocol::Action::SystemConquerred,
                ConquestData{
                    system: system.clone(),
                    fleet: fleet.clone(),
                },
                None,
            ),
            FleetArrivalOutcome::Defended { system, fleets } => protocol::Message::new(
                protocol::Action::CombatEnded,
                combat::CombatData {
                    system: system.clone(),
                    fleets: fleets.clone(),
                },
                None,
            ),
            FleetArrivalOutcome::Arrived { fleet } => protocol::Message::new(
                protocol::Action::FleetArrived,
                fleet.clone(),
                None,
            )
        }
    }
}

pub async fn generate_systems(gid: GameID) -> Result<Graph<System, ()>> {
    let mut graph = Graph::new();
    GalaxyBuilder::default()
        .cloud_population(2)
        .nb_arms(5)
        .nb_arm_bones(32)
        .slope_factor(0.4)
        .arm_slope(std::f64::consts::PI / 4.0)
        .arm_width_factor(1.0 / 24.0)
        .populate(Point { x:0f64, y:0f64 }, &mut graph);

    let node_transform = |_idx, &DataPoint { point:Point { x, y }, .. }| {
        // tout le code qui cre un systeme a partir d'un DataPoint<NodeType>
        generate_system(&gid, x, y)
    };

    // tout le code qui modifie une edge (si jamais un jour on a besoin de l'info
    let edge_transform = |_, _| ();

    Ok(graph.map(node_transform, edge_transform))
}

fn generate_system(gid: &GameID, x: f64, y: f64) -> System {
    System{
        id: SystemID(Uuid::new_v4()),
        game: gid.clone(),
        player: None,
        coordinates: Coordinates{ x, y },
        unreachable: false
    }
}

pub async fn assign_systems(players:Vec<PlayerID>, galaxy:&mut Graph<System, ()>) -> Result<()> {
    let mut min : Coordinates = Coordinates { x:f64::MAX, y:f64::MAX };
    let mut max : Coordinates = Coordinates { x:f64::MIN, y:f64::MIN };

    for sys in galaxy.node_indices().map(|idx| &galaxy[idx]) {
        min.x = min.x.min(sys.coordinates.x);
        min.y = min.y.min(sys.coordinates.y);
        max.x = max.x.max(sys.coordinates.x);
        max.y = max.y.max(sys.coordinates.y);
    }

    for player in players {
        let place = find_place(&min, &max, galaxy).await.ok_or(InternalError::SystemUnknown)?;
        place.player = Some(player);
    }

    Ok(())
}

async fn find_place<'a>(
    Coordinates { x:xmin, y:ymin }: &Coordinates,
    Coordinates { x:xmax, y:ymax }: &Coordinates,
    galaxy:& 'a mut Graph<System, ()>
)
    -> Option<& 'a mut System>
{

    let mut rng = thread_rng();
    let final_x: f64 = rng.gen_range(xmin, xmax);
    let final_y: f64 = rng.gen_range(ymin, ymax);
    let final_coord = Coordinates { x:final_x, y:final_y };

    let mut min_dist = f64::MAX;
    let mut idx = None;
    for sid in galaxy.node_indices() {
        let sys = &galaxy[sid];
        let dist = final_coord.dot(&sys.coordinates);
        if sys.player.is_none() && dist < min_dist {
            min_dist = dist;
            idx = Some(sid);
        }
    }

    Some(&mut galaxy[idx?])
}
