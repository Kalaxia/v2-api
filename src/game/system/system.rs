use actix_web::{get, web, HttpResponse};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use crate::{
    AppState,
    lib::{
        Result,
        pagination::{Paginator, new_paginated_response},
        error::{ServerError, InternalError},
        uuid::Uuid,
    },
    game::{
        faction::FactionID,
        fleet::combat,
        fleet::{
            fleet::Fleet,
            ship::ShipGroup,
        },
        game::{Game, GameOptionMapSize, GameOptionSpeed},
        player::Player,
        system::{
            building::{Building, BuildingStatus, BuildingKind},
        },
    },
    ws::protocol
};
use galaxy_rs::{Point, DataPoint};
use sqlx::{PgPool, PgConnection, pool::PoolConnection, postgres::{PgRow, PgQueryAs}, FromRow, Error, Transaction};
use sqlx_core::row::Row;
use rand::{prelude::*, distributions::{Distribution, Uniform}};

#[derive(Serialize, Deserialize, Clone)]
pub struct System {
    pub id: Uuid<System>,
    pub game: Uuid<Game>,
    pub player: Option<Uuid<Player>>,
    pub kind: SystemKind,
    pub coordinates: Coordinates,
    pub unreachable: bool
}

#[derive(Debug, Clone)]
struct MapSizeData {
    cloud_population: u64,
    nb_arms: u64,
    nb_arm_bones: u64,
    min_distance: f64,
    slope_factor: f64,
    arm_slope: f64,
    arm_width_factor: f64
}

#[derive(Serialize, Deserialize, Clone)]
pub enum SystemKind {
    BaseSystem,
    VictorySystem,
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
        fleets: HashMap<Uuid<Fleet>, Fleet>,
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
    
    pub fn as_distance_to(&self, to: &Coordinates) -> f64 {
        ((to.x - self.x).powi(2) + (to.y - self.y).powi(2)).sqrt()
    }
}

impl From<SystemKind> for i16 {
    fn from(kind: SystemKind) -> Self {
        match kind {
            SystemKind::BaseSystem => 1,
            SystemKind::VictorySystem => 2,
        }
    }
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
        let id = row.try_get("id")?;
        let game = row.try_get("game_id")?;
        let player_id = row.try_get("player_id").ok();

        Ok(System {
            id,
            game,
            player: player_id,
            kind: SystemKind::from_row(row)?,
            coordinates: Coordinates::from_row(row)?,
            unreachable: row.try_get("is_unreachable")?,
        })
    }
}

impl<'a> FromRow<'a, PgRow<'a>> for SystemKind {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(match row.try_get::<i16, _>("kind")? {
            1 => Self::BaseSystem,
            2 => Self::VictorySystem,
            _ => Self::BaseSystem,
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

                let mut fleets = self.retrieve_orbiting_fleets(db_pool).await?;

                if fleets.is_empty() || combat::engage(&mut fleet, &mut fleets, db_pool).await? == true {
                    return self.conquer(fleet, db_pool).await;
                }
                fleets.insert(fleet.id.clone(), fleet.clone());
                Ok(FleetArrivalOutcome::Defended{ fleets, system: self.clone() })
            },
            None => self.conquer(fleet, db_pool).await
        }
    }

    async fn retrieve_orbiting_fleets(&self, db_pool: &PgPool) -> Result<HashMap<Uuid<Fleet>, Fleet>> {
        let mut ids = vec![];
        // Conquest of the system by the arrived fleet
        let mut fleets: HashMap<Uuid<Fleet>, Fleet> = Fleet::find_stationed_by_system(&self.id, db_pool).await?
            .into_iter()
            .map(|f| {
                ids.push(f.id.clone());
                (f.id.clone(), f)
            })
            .collect();
        let ship_groups = ShipGroup::find_by_fleets(ids, db_pool).await?;

        for sg in ship_groups.into_iter() {
            fleets.get_mut(&sg.fleet.unwrap()).unwrap().ship_groups.push(sg.clone()); 
        }
        Ok(fleets)
    }

    pub async fn conquer(&mut self, mut fleet: Fleet, db_pool: &PgPool) -> Result<FleetArrivalOutcome> {
        fleet.change_system(self);
        Fleet::update(fleet.clone(), db_pool).await?;
        self.player = Some(fleet.player.clone());
        System::update(self.clone(), db_pool).await?;
        Ok(FleetArrivalOutcome::Conquerred{
            system: self.clone(),
            fleet: fleet,
        })
    }

    pub async fn find(sid: Uuid<System>, db_pool: &PgPool) -> Result<System> {
        sqlx::query_as("SELECT * FROM map__systems WHERE id = $1")
            .bind(sid)
            .fetch_one(db_pool).await.map_err(ServerError::if_row_not_found(InternalError::SystemUnknown))
    }

    pub async fn find_possessed(gid: Uuid<Game>, db_pool: &PgPool) -> Result<Vec<System>> {
        sqlx::query_as("SELECT * FROM map__systems WHERE game_id = $1 AND player_id IS NOT NULL")
            .bind(gid)
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_possessed_victory_systems(gid: Uuid<Game>, db_pool: &PgPool) -> Result<Vec<System>> {
        sqlx::query_as("SELECT * FROM map__systems WHERE game_id = $1 AND kind = $2 AND player_id IS NOT NULL")
            .bind(gid)
            .bind(i16::from(SystemKind::VictorySystem))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_all(gid: &Uuid<Game>, limit: i64, offset: i64, db_pool: &PgPool) -> Result<Vec<System>> {
        sqlx::query_as("SELECT * FROM map__systems WHERE game_id = $1 LIMIT $2 OFFSET $3")
            .bind(*gid)
            .bind(limit)
            .bind(offset)
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn count_by_faction(gid: Uuid<Game>, db_pool: &PgPool) -> Result<Vec<SystemDominion>> {
        sqlx::query_as(
            "SELECT f.id as faction_id, COUNT(s.*) as nb_systems FROM map__systems s
            INNER JOIN player__players p ON s.player_id = p.id
            INNER JOIN faction__factions f ON p.faction_id = f.id
            WHERE s.game_id = $1
            GROUP BY f.id")
        .bind(gid)
        .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn count(gid: Uuid<Game>, db_pool: &PgPool) -> u32 {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM map__systems WHERE game_id = $1")
            .bind(gid)
            .fetch_one(db_pool).await.expect("Could not retrieve systems");
        count.0 as u32
    }

    pub async fn create(s: System,  tx: &mut Transaction<PoolConnection<PgConnection>>) -> Result<u64> {
        sqlx::query("INSERT INTO map__systems (id, game_id, player_id, kind, coord_x, coord_y, is_unreachable) VALUES($1, $2, $3, $4, $5, $6, $7)")
            .bind(s.id)
            .bind(s.game)
            .bind(s.player)
            .bind(i16::from(s.kind))
            .bind(s.coordinates.x)
            .bind(s.coordinates.y)
            .bind(s.unreachable)
            .execute(tx).await.map_err(ServerError::from)
    }

    pub async fn update(s: System, db_pool: &PgPool) -> Result<u64> {
        sqlx::query("UPDATE map__systems SET player_id = $1, is_unreachable = $2 WHERE id = $3")
            .bind(s.player)
            .bind(s.unreachable)
            .bind(s.id)
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

pub async fn generate_systems(gid: Uuid<Game>, map_size: GameOptionMapSize) -> Result<Vec<System>> {
    let graph = map_size.to_galaxy_builder().build(Point { x:0f64, y:0f64 }).expect("Failed to generate the galaxy map");

    let mut probability: f64 = 0.5;

    Ok(graph.into_points().map(|DataPoint { point:Point { x, y }, .. }| {
        let (system, prob) = generate_system(&gid, x, y, probability);
        probability = prob;
        system
    }).collect())
}

fn generate_system(gid: &Uuid<Game>, x: f64, y: f64, probability: f64) -> (System, f64) {
    let (kind, prob) = generate_system_kind(x, y, probability);
    (System{
        id: Uuid::new(),
        game: gid.clone(),
        player: None,
        kind: kind,
        coordinates: Coordinates{ x, y },
        unreachable: false
    }, prob)
}

fn generate_system_kind(x: f64, y: f64, probability: f64) -> (SystemKind, f64) {
    let mut rng = rand::thread_rng();
    let rand: f64 = rng.gen_range((x.abs() + y.abs()) / 2.0, x.abs() + y.abs() + 1.0);

    if rand <= probability {
        return (SystemKind::VictorySystem, 0.5);
    }
    (SystemKind::BaseSystem, probability + 0.1)
}

pub async fn assign_systems(players:&Vec<Player>, galaxy:&mut Vec<System>) -> Result<()> {

    const GRID_SIZE : usize = 16;
    const EXCLUSION : usize = 1;

    let mut rng = thread_rng();
    let mut faction_cell = HashMap::new();
    let mut taken : [[bool;GRID_SIZE];GRID_SIZE] = [[false;GRID_SIZE];GRID_SIZE];
    let mut min : Coordinates = Coordinates { x: std::f64::MAX, y: std::f64::MAX };
    let mut max : Coordinates = Coordinates { x: std::f64::MIN, y: std::f64::MIN };

    let grid_range = Uniform::from(0..GRID_SIZE);

    for sys in galaxy.iter() {
        min.x = min.x.min(sys.coordinates.x);
        min.y = min.y.min(sys.coordinates.y);
        max.x = max.x.max(sys.coordinates.x);
        max.y = max.y.max(sys.coordinates.y);
    }

    let cell_w = (max.x - min.x) / GRID_SIZE as f64;
    let cell_h = (max.y - min.y) / GRID_SIZE as f64;

    for player in players {
        // Take the zone assigned to the player's faction
        // Assigning a new zone when encountering a new faction
        let (cell_min, cell_max) = faction_cell
            .entry(player.faction.unwrap())
            .or_insert_with(|| {
                let mut cell_x = grid_range.sample(&mut rng);
                let mut cell_y = grid_range.sample(&mut rng);
                while taken[cell_x][cell_y] {
                    cell_x = grid_range.sample(&mut rng);
                    cell_y = grid_range.sample(&mut rng);
                }

                // make the place AND its neighbours in a zone which width is defined by the
                // EXCLUSION constant not usable anymore
                for i in cell_x.saturating_sub(EXCLUSION)..=(cell_x+EXCLUSION).min(GRID_SIZE-1) {
                    for j in cell_y.saturating_sub(EXCLUSION)..=(cell_y+EXCLUSION).min(GRID_SIZE-1) {
                        taken[i][j] = true;
                    }
                }

                // the (x, y) coordinates of the topleft corner of the chosen cell
                let x = min.x + cell_x as f64 * cell_w;
                let y = min.y + cell_y as f64 * cell_h;

                (Coordinates { x, y }, Coordinates { x: x + cell_w, y: y + cell_h })
            });

        // find a place for the player in its faction zone
        let place = find_place(cell_min, cell_max, galaxy).await.ok_or(InternalError::SystemUnknown)?;
        place.player = Some(player.id);
    }

    Ok(())
}

async fn find_place<'a>(
    Coordinates { x:xmin, y:ymin }: &Coordinates,
    Coordinates { x:xmax, y:ymax }: &Coordinates,
    galaxy:& 'a mut Vec<System>
)
    -> Option<& 'a mut System>
{
    let mut rng = thread_rng();
    let final_x: f64 = rng.gen_range(xmin, xmax);
    let final_y: f64 = rng.gen_range(ymin, ymax);
    let final_coord = Coordinates { x:final_x, y:final_y };

    let mut min_dist = std::f64::MAX;
    let mut idx = None;
    for (sid, sys) in galaxy.iter().enumerate() {
        let dist = final_coord.as_distance_to(&sys.coordinates);
        if sys.player.is_none() && dist < min_dist {
            min_dist = dist;
            idx = Some(sid);
        }
    }

    idx.map(move |id| &mut galaxy[id])
}

#[get("/")]
pub async fn get_systems(state: web::Data<AppState>, info: web::Path<(Uuid<Game>,)>, pagination: web::Query<Paginator>)
    -> Result<HttpResponse>
{
    Ok(new_paginated_response(
        pagination.limit,
        pagination.page,
        System::count(info.0.clone(), &state.db_pool).await.into(),
        System::find_all(&info.0, pagination.limit, (pagination.page - 1) * pagination.limit, &state.db_pool).await?,
    ))
}

pub async fn init_player_systems(systems: &Vec<System>, game_speed: GameOptionSpeed, db_pool: &PgPool) -> Result<()> {
    let building_data = BuildingKind::Shipyard.to_data();
    let mut tx = db_pool.begin().await?;

    for s in systems.iter() {
        if s.player.is_none() {
            continue;
        }

        let mut building = Building::new(s.id, BuildingKind::Shipyard, building_data, game_speed);
        building.status = BuildingStatus::Operational;
        building.built_at = building.created_at;

        Building::create(building, &mut tx).await?;
    }
    tx.commit().await?;
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_as_distance_to() {
        assert_eq!(2.8284271247461903, Coordinates{
            x: 2.0,
            y: 2.0,
        }.as_distance_to(&Coordinates{
            x: 4.0,
            y: 4.0
        }));
    }
}
