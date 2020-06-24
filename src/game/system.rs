use uuid::Uuid;
use futures::future::join_all;
use serde::{Serialize, Deserialize};
use std::collections::{HashMap};
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
use sqlx::{PgPool, postgres::{PgRow, PgQueryAs}, FromRow, Error};
use sqlx_core::row::Row;

#[derive(Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct SystemID(pub Uuid);

#[derive(Serialize, Deserialize, Clone)]
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

#[derive(Serialize, Deserialize, Clone)]
pub struct Coordinates {
    pub x: u16,
    pub y: u16
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

impl From<SystemID> for Uuid {
    fn from(sid: SystemID) -> Self { sid.0 }
}

impl From<i32> for Coordinates {
    fn from(c: i32) -> Self {
        Self{
            x: ((c >> 16) & 0xff) as u16,
            y: ((c >> 0) & 0xff) as u16,
        }
    }
}

impl From<Coordinates> for i32 {
    fn from(Coordinates{ x, y }: Coordinates) -> Self {
        ((x as i32) << 16) | ((y as i32) & 0xffff)
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
            coordinates: Coordinates::from(row.try_get::<i32, _>("coordinates")?),
            unreachable: row.try_get("is_unreachable")?,
        })
    }
}

impl<'a> FromRow<'a, PgRow<'a>> for SystemDominion {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        let id : i32 = row.try_get("faction_id")?;

        Ok(SystemDominion {
            faction_id: FactionID(id as u8),
            nb_systems: row.try_get("nb_systems").map(u32::into)?,
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
                    return Ok(FleetArrivalOutcome::Arrived{ fleet });
                }
                // Conquest of the system by the arrived fleet
                let mut fleets: HashMap<FleetID, Fleet> = Fleet::find_stationed_by_system(&self.id, db_pool).await
                    .into_iter()
                    .map(|f| (f.id.clone(), f))
                    .collect();
                if fleets.is_empty() || combat::engage(&mut fleet, &mut fleets) == true {
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
        self.player = Some(fleet.player.clone());
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

    pub async fn create(s: System, db_pool: &PgPool) -> Result<u64> {
        sqlx::query("INSERT INTO map__systems (id, game_id, player_id, coordinates, is_unreachable) VALUES($1,$2, $3, $4, $5)")
            .bind(Uuid::from(s.id))
            .bind(Uuid::from(s.game))
            .bind(s.player.map(Uuid::from))
            .bind(i32::from(s.coordinates))
            .bind(s.unreachable)
            .execute(db_pool).await.map_err(ServerError::from)
    }

    pub async fn update(s: System, db_pool: &PgPool) -> Result<u64> {
        sqlx::query("UPDATE map__systems SET player_id = $1, is_unreachable = $2 WHERE id = $3")
            .bind(s.player.map(Uuid::from))
            .bind(s.unreachable)
            .bind(Uuid::from(s.id))
            .execute(db_pool).await.map_err(ServerError::from)
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

pub async fn generate_systems(gid: GameID, db_pool: &PgPool) -> HashMap<SystemID, System> {
    let mut systems = HashMap::new();
    for y in 0..MAP_SIZE {
        let mut queries = vec![];
        for x in 0..MAP_SIZE {
            let system = generate_system(&gid, x, y);
            queries.push(System::create(system.clone(), db_pool));
            systems.insert(system.id.clone(), system);
        }
        println!("{:?}", queries.len());
        join_all(queries).await.into_iter().for_each(|r| { println!("{:?}", r); });
        println!("ok");
    }
    systems
}

fn generate_system(gid: &GameID, x: u16, y: u16) -> System {
    System{
        id: SystemID(Uuid::new_v4()),
        game: gid.clone(),
        player: None,
        coordinates: Coordinates{ x, y },
        unreachable: false
    }
}

pub async fn assign_systems(gid: GameID, db_pool: &PgPool) -> Result<()> {
    let mut placed_per_faction: HashMap<FactionID, u16> = HashMap::new();
    let players = Player::find_by_game(gid, db_pool).await;
    let mut systems : HashMap<SystemID, System> = System::find_all(&gid, db_pool).await
        .into_iter()
        .map(|s| (s.id.clone(), s))
        .collect();

    for player in players {
        let fid = player.faction.unwrap().clone();
        let i = placed_per_faction.entry(fid).or_insert(0);
        let place = find_place(fid, *i, &systems);
        *i += 1;

        if let Some(place) = place {
            // legitimate use of unwrap, because we KNOW `place` IS an existing system id
            // if it is Some()
            let mut s = systems.get_mut(&place).unwrap();
            s.player = Some(player.id);
            System::update(s.clone(), db_pool).await?;
        } else {
            // else do something to handle the non-placed player
            // here we put unreachable!() because it is normaly the case.
            unreachable!()
        }
    }
    Ok(())
}

fn find_place(fid: FactionID, i: u16, systems: &HashMap<SystemID, System>) -> Option<SystemID> {
    // Each faction is associated to a side of the grid
    let coordinates_check: & dyn Fn(u16, u16, u16) -> bool = match fid {
        FactionID(1) => &|i, x, y| x > 0 || y < i,
        FactionID(2) => &|i, x, y| x < MAP_SIZE - 1 || y < i,
        FactionID(3) => &|i, x, y| y > 0 || x < i,
        FactionID(4) => &|i, x, y| y < MAP_SIZE - 1 || x < i,
        _ => unimplemented!() // better than "None" because normaly this function is total
    };
    for (sid, system) in systems {
        if coordinates_check(i, system.coordinates.x, system.coordinates.y) || system.player != None {
            continue;
        }
        return Some(sid.clone());
    }
    None
}
