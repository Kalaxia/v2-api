use crate::{
    lib::{
        Result,
        error::ServerError
    },
    game::{
        game::game::GameID,
        player::player::{PlayerID, Player},
        ship::model::ShipModelCategory,
    }
};
use uuid::Uuid;
use sqlx::{PgPool, postgres::{PgRow, PgQueryAs}, Executor, FromRow, Error, Postgres, types::Json};
use sqlx_core::row::Row;
use std::collections::HashMap;
use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct GameRankings {
    pub wealth: Vec<(PlayerID, usize)>,
    pub destroyed_ships: Vec<(PlayerID, i32, HashMap<ShipModelCategory, i32>)>,
    pub lost_ships: Vec<(PlayerID, i32, HashMap<ShipModelCategory, i32>)>,
    pub successful_conquests: Vec<(PlayerID, i32)>,
    pub lost_systems: Vec<(PlayerID, i32)>
}

#[derive(Serialize, Clone)]
pub struct PlayerRanking {
    pub player: PlayerID,
    pub destroyed_ships: HashMap<ShipModelCategory, i32>,
    pub destroyed_ships_score: i32,
    pub lost_ships: HashMap<ShipModelCategory, i32>,
    pub lost_ships_score: i32,
    pub successful_conquests: i32,
    pub lost_systems: i32
}

impl<'a> FromRow<'a, PgRow<'a>> for PlayerRanking {
    fn from_row(row: &PgRow) -> std::result::Result<Self, Error> {
        Ok(PlayerRanking {
            player: row.try_get("player_id").map(PlayerID)?,
            destroyed_ships: (&*row.try_get::<Json<HashMap<ShipModelCategory, i32>>, _>("destroyed_ships")?).clone(),
            destroyed_ships_score: row.try_get("destroyed_ships_score")?,
            lost_ships: (&*row.try_get::<Json<HashMap<ShipModelCategory, i32>>, _>("lost_ships")?).clone(),
            lost_ships_score: row.try_get("lost_ships_score")?,
            successful_conquests: row.try_get("successful_conquests")?,
            lost_systems: row.try_get("lost_systems")?,
        })
    }
}

impl PlayerRanking {
    pub fn new(player_id: PlayerID) -> Self {
        Self {
            player: player_id,
            destroyed_ships: HashMap::new(),
            destroyed_ships_score: 0,
            lost_ships: HashMap::new(),
            lost_ships_score: 0,
            successful_conquests: 0,
            lost_systems: 0
        }
    }

    pub async fn find_by_game(gid: GameID, db_pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as("SELECT pr.* FROM player__rankings pr INNER JOIN player__players p ON p.id = pr.player_id WHERE p.game_id = $1")
            .bind(Uuid::from(gid))
            .fetch_all(db_pool).await.map_err(ServerError::from)
    }

    pub async fn increment_lost_systems<E>(pid: PlayerID, exec: &mut E) -> Result<u64> 
        where E: Executor<Database = Postgres> {
        sqlx::query("UPDATE player__rankings SET lost_systems = lost_systems + 1 WHERE player_id = $1")
            .bind(Uuid::from(pid))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn increment_successful_conquests<E>(pid: PlayerID, exec: &mut E) -> Result<u64> 
        where E: Executor<Database = Postgres> {
        sqlx::query("UPDATE player__rankings SET successful_conquests = successful_conquests + 1 WHERE player_id = $1")
            .bind(Uuid::from(pid))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn add_lost_ships<E>(player_id: PlayerID, ship_category: ShipModelCategory, quantity: i32, strength: u16, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
            Self::add_ships("lost_ships", player_id, ship_category, quantity, strength, exec).await
        }

    pub async fn add_destroyed_ships<E>(player_id: PlayerID, ship_category: ShipModelCategory, quantity: i32, strength: u16, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
            Self::add_ships("destroyed_ships", player_id, ship_category, quantity, strength, exec).await
        }

    pub async fn add_ships<E>(column: &str, player_id: PlayerID, ship_category: ShipModelCategory, quantity: i32, strength: u16, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query(&format!("UPDATE player__rankings SET {data} = jsonb_set({data}, '{{{category}}}', to_jsonb(COALESCE(({data}::json->>'{category}')::int, 0) + $2)), {score} = {score} + $3 WHERE player_id = $1", data=column, category=ship_category.to_string().to_lowercase(), score=format!("{}_score", column)))
            .bind(Uuid::from(player_id))
            .bind(quantity)
            .bind(i32::from(strength))
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
    
    pub async fn insert<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("INSERT INTO player__rankings (player_id, destroyed_ships, destroyed_ships_score, lost_ships, lost_ships_score, successful_conquests, lost_systems) VALUES($1, $2, $3, $4, $5, $6, $7)")
            .bind(Uuid::from(self.player))
            .bind(Json(&self.destroyed_ships))
            .bind(self.destroyed_ships_score)
            .bind(Json(&self.lost_ships))
            .bind(self.lost_ships_score)
            .bind(self.successful_conquests)
            .bind(self.lost_systems)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }

    pub async fn update<E>(&self, exec: &mut E) -> Result<u64>
        where E: Executor<Database = Postgres> {
        sqlx::query("UPDATE player__rankings SET destroyed_ships = $2, destroyed_ships_score = $3, lost_ships = $4, lost_ships_score = $5, successful_conquests = $6, lost_systems = $7 WHERE player_id = $1")
            .bind(Uuid::from(self.player))
            .bind(Json(&self.destroyed_ships))
            .bind(self.destroyed_ships_score)
            .bind(Json(&self.lost_ships))
            .bind(self.lost_ships_score)
            .bind(self.successful_conquests)
            .bind(self.lost_systems)
            .execute(&mut *exec).await.map_err(ServerError::from)
    }
}

pub fn generate_game_rankings(players: Vec<Player>, player_rankings: Vec<PlayerRanking>) -> GameRankings {
    let mut game_rankings = GameRankings {
        wealth: vec![],
        destroyed_ships: vec![],
        lost_ships: vec![],
        successful_conquests: vec![],
        lost_systems: vec![]
    };

    for player in players {
        game_rankings.wealth.push((player.id, player.wallet));
    }

    for ranking in player_rankings {
        game_rankings.destroyed_ships.push((ranking.player.clone(), ranking.destroyed_ships_score, ranking.destroyed_ships));
        game_rankings.lost_ships.push((ranking.player.clone(), ranking.lost_ships_score, ranking.lost_ships));
        game_rankings.successful_conquests.push((ranking.player.clone(), ranking.successful_conquests));
        game_rankings.lost_systems.push((ranking.player.clone(), ranking.lost_systems));
    }
    
    game_rankings.wealth.sort_by_key(|k| k.1);
    game_rankings.wealth.reverse();
    game_rankings.destroyed_ships.sort_by_key(|k| k.1);
    game_rankings.destroyed_ships.reverse();
    game_rankings.lost_ships.sort_by_key(|k| k.1);
    game_rankings.successful_conquests.sort_by_key(|k| k.1);
    game_rankings.successful_conquests.reverse();
    game_rankings.lost_systems.sort_by_key(|k| k.1);

    game_rankings
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use crate::{
        lib::{
            time::Time,
        },
        game::{
            fleet::{
                fleet::{Fleet, FleetID},
                formation::{FleetFormation},
                squadron::{FleetSquadron, FleetSquadronID},
            },
            ship::model::ShipModelCategory,
            system::system::{SystemID},
            player::player::{PlayerID}
        }
    };

    #[test]
    fn test_generate_game_rankings() {
        let players = get_players_mock();
        let player_rankings = get_player_rankings_mock();

        let game_rankings = generate_game_rankings(players, player_rankings);

        assert_eq!(vec![
            2200,
            1800,
            1220,
            1000,
            350,
            0
        ], game_rankings.wealth.iter().map(|w| w.1).collect::<Vec<usize>>());

        assert_eq!(
            vec![50, 30, 20, 18, 15, 0],
            game_rankings.destroyed_ships.iter().map(|ds| ds.1).collect::<Vec<i32>>()
        );

        assert_eq!(
            vec![0, 2, 10, 15, 40, 48],
            game_rankings.lost_ships.iter().map(|ls| ls.1).collect::<Vec<i32>>()
        );

        assert_eq!(
            vec![8, 6, 5, 5, 4, 4],
            game_rankings.successful_conquests.iter().map(|sc| sc.1).collect::<Vec<i32>>()
        );

        assert_eq!(
            vec![0, 1, 2, 4, 5, 9],
            game_rankings.lost_systems.iter().map(|ls| ls.1).collect::<Vec<i32>>()
        );
    }

    fn get_players_mock() -> Vec<Player> {
        vec![
            get_player_mock(1000),
            get_player_mock(350),
            get_player_mock(2200),
            get_player_mock(1220),
            get_player_mock(1800),
            get_player_mock(0),
        ]
    }

    fn get_player_rankings_mock() -> Vec<PlayerRanking> {
        vec![
            get_player_ranking_mock(20, 10, 4, 4),
            get_player_ranking_mock(15, 0, 5, 2),
            get_player_ranking_mock(30, 40, 5, 5),
            get_player_ranking_mock(50, 48, 8, 9),
            get_player_ranking_mock(0, 2, 6, 1),
            get_player_ranking_mock(18, 15, 4, 0),
        ]
    }

    fn get_player_ranking_mock(destroyed_ships_score: i32, lost_ships_score: i32, successful_conquests: i32, lost_systems: i32) -> PlayerRanking {
        PlayerRanking{
            player: PlayerID(Uuid::new_v4()),
            destroyed_ships: HashMap::new(),
            destroyed_ships_score,
            lost_ships: HashMap::new(),
            lost_ships_score,
            successful_conquests,
            lost_systems
        }
    }

    fn get_player_mock(wallet: usize) -> Player {
        Player {
            id: PlayerID(Uuid::new_v4()),
            wallet,
            username: String::from(""),
            game: None,
            lobby: None,
            faction: None,
            ready: true,
            is_connected: true,
        }
    }
}