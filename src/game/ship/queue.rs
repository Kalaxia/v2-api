use actix_web::{get, post, web, HttpResponse};
use sqlx::{PgPool, Executor, postgres::{PgRow, PgQueryAs}, FromRow, Error, Postgres};
use sqlx_core::row::Row;
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use crate::{
    task,
    lib::{
        Result,
        auth::Claims,
        error::{ServerError, InternalError},
        time::Time,
    },
    game::{
        player::{Player},
        fleet::{
            fleet::FleetID,
            formation::FleetFormation,
            squadron::FleetSquadron,
        },
        game::{
            game::{Game, GameID},
            option::GameOptionSpeed,
            server::{GameServer, GameServerTask},
        },
        ship::{
            model::ShipModelCategory,
            squadron::{Squadron},
        },
        system::{
            building::{Building, BuildingKind},
            system::{SystemID, System},
        },
    },
    ws::protocol,
    AppState,
};
use futures::join;
use futures::executor::block_on;

#[derive(Debug, Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy)]
pub struct ShipQueueID(pub Uuid);

#[derive(Debug, Serialize, Clone)]
pub struct ShipQueue {
    pub id: ShipQueueID,
    pub system: SystemID,
    pub category: ShipModelCategory,
    pub quantity: u16,
    pub assigned_fleet: Option<String>,
    pub created_at: Time,
    pub started_at: Time,
    pub finished_at: Time,
}

#[derive(serde::Deserialize)]
pub struct ShipQuantityData {
    pub category: ShipModelCategory,
    pub quantity: usize
}

impl From<ShipQueueID> for Uuid {
    fn from(sqid: ShipQueueID) -> Self { sqid.0 }
}

impl<'a> FromRow<'a, PgRow<'a>> for ShipQueue {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(ShipQueue {
            id: row.try_get("id").map(ShipQueueID)?,
            system: row.try_get("system_id").map(SystemID)?,
            category: row.try_get("category")?,
            quantity: row.try_get::<i32, _>("quantity")? as u16,
            assigned_fleet: row.try_get("assigned_fleet")?,
            created_at: row.try_get("created_at")?,
            started_at: row.try_get("started_at")?,
            finished_at: row.try_get("finished_at")?,
        })
    }
}

impl GameServerTask for ShipQueue {
    fn get_task_id(&self) -> String {
        self.id.0.to_string()
    }

    fn get_task_end_time(&self) -> Time {
        self.finished_at
    }
}

impl ShipQueue {
    pub async fn find(sqid: ShipQueueID, db_pool: &PgPool) -> Result<Self> {
        sqlx::query_as("SELECT * FROM system__ship_queues WHERE id = $1")
            .bind(Uuid::from(sqid))
            .fetch_one(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_by_system(sid: SystemID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT * FROM system__ship_queues WHERE system_id = $1 ORDER BY finished_at DESC")
            .bind(Uuid::from(sid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn find_last(sid: SystemID, db_pool: &PgPool) -> Result<Self> {
        sqlx::query_as("SELECT * FROM system__ship_queues WHERE system_id = $1 ORDER BY finished_at DESC LIMIT 1")
            .bind(Uuid::from(sid))
            .fetch_one(db_pool).await.map_err(ServerError::from)
    }

    pub async fn count_assigned_ships(assigned_fleet: &str, category: ShipModelCategory, db_pool: &PgPool) -> Result<u32> {
        let count: (i64,) = sqlx::query_as("SELECT COALESCE(SUM(quantity), 0) FROM system__ship_queues WHERE assigned_fleet = $1 AND category = $2")
            .bind(assigned_fleet)
            .bind(category)
            .fetch_one(db_pool).await.map_err(ServerError::from)?;
        Ok(count.0 as u32)
    }

    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO system__ship_queues (id, system_id, category, quantity, assigned_fleet, created_at, started_at, finished_at) VALUES($1, $2, $3, $4, $5, $6, $7, $8)")
            .bind(Uuid::from(self.id))
            .bind(Uuid::from(self.system))
            .bind(self.category)
            .bind(self.quantity as i32)
            .bind(self.assigned_fleet.as_ref())
            .bind(self.created_at)
            .bind(self.started_at)
            .bind(self.finished_at)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn remove<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("DELETE FROM system__ship_queues WHERE id = $1")
            .bind(Uuid::from(self.id))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn produce(&self, server: &GameServer) -> Result<()> {
        let player = Player::find_system_owner(self.system.clone(), &server.state.db_pool).await?;
        let mut tx = server.state.db_pool.begin().await?;

        if let Some(assigned_fleet) = self.assigned_fleet.clone() {
            let fleet_data: Vec<&str> = assigned_fleet.split(':').collect();
            let fleet_id = FleetID(Uuid::parse_str(fleet_data[0]).map_err(ServerError::from)?);
            let formation: FleetFormation = fleet_data[1].parse()?;
            FleetSquadron::assign_existing(
                fleet_id,
                formation,
                self.category,
                self.quantity,
                &server.state.db_pool
            ).await?;
        } else {
            Squadron::assign_existing(
                self.system,
                self.category,
                self.quantity as i32,
                &server.state.db_pool
            ).await?;
        }
        self.remove(&mut tx).await?;

        tx.commit().await?;

        server.ws_send(&player.id, protocol::Message::new(
            protocol::Action::ShipQueueFinished,
            self.clone(),
            None,
        ));

        Ok(())
    }

    pub async fn schedule(
        player: &mut Player,
        sid: SystemID,
        category: ShipModelCategory,
        mut quantity: u16,
        only_affordable: bool,
        assigned_fleet: Option<String>,
        game_speed: GameOptionSpeed,
        db_pool: &PgPool
    ) -> Result<Option<ShipQueue>> {
        let has_shipyard = Building::count_by_kind_and_system(BuildingKind::Shipyard, sid, &db_pool).await? > 0;
        if !has_shipyard {
            return Err(InternalError::Conflict.into());
        }

        let ship_model = category.to_data();
        if only_affordable {
            let affordable_quantity = (player.wallet / ship_model.cost as usize) as u16;
            if affordable_quantity < 1 {
                return Ok(None);
            } else if affordable_quantity < quantity  {
                quantity = affordable_quantity;
            }
        }
        player.spend(ship_model.cost as usize * quantity.clone() as usize)?;
        
        let starts_at = ShipQueue::find_last(sid.clone(), &db_pool).await.ok().map_or(Time::now(), |sq| sq.finished_at);

        let ship_queue = ShipQueue{
            id: ShipQueueID(Uuid::new_v4()),
            system: sid,
            category: category.clone(),
            quantity: quantity.clone(),
            assigned_fleet,
            created_at: Time::now(),
            started_at: starts_at.clone(),
            finished_at: ship_model.compute_construction_deadline(quantity, starts_at, game_speed),
        };
        let mut tx = db_pool.begin().await?;
        ship_queue.insert(&mut tx).await?;
        player.update(&mut tx).await?;
        tx.commit().await?;

        Ok(Some(ship_queue))
    }
}


#[post("/")]
pub async fn add_ship_queue(
    state: web::Data<AppState>,
    info: web::Path<(GameID, SystemID)>,
    json_data: web::Json<ShipQuantityData>,
    claims: Claims
) -> Result<HttpResponse> {
    let (g, s, p) = join!(
        Game::find(info.0, &state.db_pool),
        System::find(info.1, &state.db_pool),
        Player::find(claims.pid, &state.db_pool),
    );
    let game = g?;
    let system = s?;
    let mut player = p?;

    if system.player.clone() != Some(player.id.clone()) {
        return Err(InternalError::AccessDenied.into());
    }
    let ship_queue = ShipQueue::schedule(
        &mut player,
        system.id,
        json_data.category,
        json_data.quantity as u16,
        false,
        None,
        game.game_speed,
        &state.db_pool
    ).await?.unwrap();

    let sq = ship_queue.clone();
    state.games().get(&info.0).unwrap().do_send(task!(sq -> move |gs: &GameServer| block_on(sq.produce(gs))));

    Ok(HttpResponse::Created().json(ship_queue))
}

#[get("/")]
pub async fn get_ship_queues(state: web::Data<AppState>, info: web::Path<(GameID, SystemID)>, claims: Claims)
    -> Result<HttpResponse>
{
    let (s, p) = futures::join!(
        System::find(info.1, &state.db_pool),
        Player::find(claims.pid, &state.db_pool),
    );
    let system = s?;
    let player = p?;

    if system.player.clone() != Some(player.id.clone()) {
        return Err(InternalError::AccessDenied.into());
    }
    Ok(HttpResponse::Ok().json(ShipQueue::find_by_system(system.id, &state.db_pool).await?))
}
