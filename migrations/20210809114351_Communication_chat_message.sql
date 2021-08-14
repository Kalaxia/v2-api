-- Add migration script here
CREATE TABLE IF NOT EXISTS communication__chat__messages(
    id UUID PRIMARY KEY,
    author_id UUID NOT NULL REFERENCES player__players(id) ON DELETE CASCADE,
    faction_id INT NOT NULL REFERENCES faction__factions(id),
    content VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);