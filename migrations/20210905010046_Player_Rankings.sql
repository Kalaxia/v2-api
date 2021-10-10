-- Add migration script here
CREATE TABLE IF NOT EXISTS player__rankings(
    player_id UUID NOT NULL,
    destroyed_ships JSONB NOT NULL,
    destroyed_ships_score INT NOT NULL,
    lost_ships JSONB NOT NULL,
    lost_ships_score INT NOT NULL,
    successful_conquests INT NOT NULL,
    lost_systems INT NOT NULL
);
ALTER TABLE player__rankings ADD CONSTRAINT player_fkey FOREIGN KEY (player_id) REFERENCES player__players (id) ON DELETE CASCADE;