DROP TABLE IF EXISTS lobby__lobbies CASCADE;
DROP TABLE IF EXISTS fleet__fleets CASCADE;
DROP TABLE IF EXISTS map__systems CASCADE;
DROP TABLE IF EXISTS game__games CASCADE;
DROP TABLE IF EXISTS game__factions CASCADE;
DROP TABLE IF EXISTS player__players CASCADE;
DROP TABLE IF EXISTS faction__factions CASCADE;

CREATE TABLE faction__factions(
    id SERIAL PRIMARY KEY,
    name VARCHAR(25) NOT NULL,
    color INT NOT NULL
);
CREATE TABLE IF NOT EXISTS lobby__lobbies(
    id UUID PRIMARY KEY,
    owner_id UUID
);
CREATE TABLE IF NOT EXISTS player__players(
    id UUID PRIMARY KEY,
    faction_id INT REFERENCES faction__factions(id) ON DELETE CASCADE,
    game_id UUID,
    lobby_id UUID,
    username VARCHAR(60) NOT NULL DEFAULT '',
    wallet INT NOT NULL DEFAULT 0,
    is_ready BOOLEAN NOT NULL DEFAULT false,
    is_connected BOOLEAN NOT NULL DEFAULT true
);
CREATE TABLE IF NOT EXISTS game__games(
    id UUID PRIMARY KEY
);
CREATE TABLE IF NOT EXISTS game__factions(
    faction_id INT NOT NULL,
    game_id UUID NOT NULL,
    victory_points SMALLINT NOT NULL
);
CREATE TABLE IF NOT EXISTS map__systems(
	id UUID PRIMARY KEY,
	game_id UUID NOT NULL,
	player_id UUID,
    kind SMALLINT NOT NULL,
	coord_x DOUBLE PRECISION NOT NULL,
    coord_y DOUBLE PRECISION NOT NULL,
	is_unreachable BOOLEAN NOT NULL
);
CREATE TABLE IF NOT EXISTS fleet__fleets(
	id UUID PRIMARY KEY,
	system_id UUID NOT NULL,
	destination_id UUID,
	player_id UUID NOT NULL,
	nb_ships INT NOT NULL
);

ALTER TABLE player__players ADD CONSTRAINT game_fkey FOREIGN KEY (game_id) REFERENCES game__games (id) ON DELETE SET NULL;
ALTER TABLE player__players ADD CONSTRAINT faction_fkey FOREIGN KEY (faction_id) REFERENCES faction__factions (id) ON DELETE CASCADE;
ALTER TABLE player__players ADD CONSTRAINT lobby_fkey FOREIGN KEY (lobby_id) REFERENCES lobby__lobbies (id) ON DELETE SET NULL;
ALTER TABLE lobby__lobbies ADD CONSTRAINT owner_fkey FOREIGN KEY (owner_id) REFERENCES player__players (id) ON DELETE SET NULL;
ALTER TABLE map__systems ADD CONSTRAINT game_fkey FOREIGN KEY (game_id) REFERENCES game__games (id) ON DELETE CASCADE;
ALTER TABLE map__systems ADD CONSTRAINT player_fkey FOREIGN KEY (player_id) REFERENCES player__players (id) ON DELETE SET NULL;
ALTER TABLE fleet__fleets ADD CONSTRAINT system_fkey FOREIGN KEY (system_id) REFERENCES map__systems (id) ON DELETE CASCADE;
ALTER TABLE fleet__fleets ADD CONSTRAINT destination_fkey FOREIGN KEY (destination_id) REFERENCES map__systems (id) ON DELETE SET NULL;
ALTER TABLE fleet__fleets ADD CONSTRAINT player_fkey FOREIGN KEY (player_id) REFERENCES player__players (id) ON DELETE CASCADE;
ALTER TABLE game__factions ADD CONSTRAINT faction_fkey FOREIGN KEY (faction_id) REFERENCES faction__factions (id) ON DELETE CASCADE;
ALTER TABLE game__factions ADD CONSTRAINT game_fkey FOREIGN KEY (game_id) REFERENCES game__games (id) ON DELETE CASCADE;

INSERT INTO faction__factions(id, name, color) VALUES(1,'Kalankar',-2469888),(2,'Valkar',4227280),(3,'Adranite',-803201024);