-- Players we have imported from chess.com. Usernames are stored lowercased.
CREATE TABLE players (
  username        TEXT PRIMARY KEY,
  player_id       BIGINT,
  joined          TIMESTAMPTZ,
  last_import_at  TIMESTAMPTZ,
  ratings         JSONB NOT NULL DEFAULT '{}'::jsonb   -- {"bullet": 1500, "blitz": ..., "rapid": ..., "daily": ...}
);

-- One row per monthly archive we have fetched. Past months never change, so a complete
-- archive is skipped (or revalidated with its ETag) on the next import.
CREATE TABLE archives (
  username    TEXT NOT NULL REFERENCES players(username) ON DELETE CASCADE,
  year        INT  NOT NULL,
  month       INT  NOT NULL,
  etag        TEXT,
  fetched_at  TIMESTAMPTZ,
  game_count  INT NOT NULL DEFAULT 0,
  complete    BOOLEAN NOT NULL DEFAULT false,
  PRIMARY KEY (username, year, month)
);

CREATE TABLE games (
  id               TEXT PRIMARY KEY,                       -- the chess.com game url
  username         TEXT NOT NULL REFERENCES players(username) ON DELETE CASCADE,
  color            TEXT NOT NULL CHECK (color IN ('white', 'black')),
  time_class       TEXT NOT NULL,                          -- bullet | blitz | rapid | daily
  time_control     TEXT,
  rules            TEXT NOT NULL,                          -- only 'chess' is indexed
  rated            BOOLEAN NOT NULL,
  end_time         TIMESTAMPTZ NOT NULL,
  result           TEXT NOT NULL,                          -- raw chess.com result code for this player
  opponent         TEXT,
  player_rating    INT,
  opponent_rating  INT,
  eco              TEXT,
  ply_count        INT NOT NULL DEFAULT 0,
  pgn              TEXT NOT NULL                           -- kept so the index can be rebuilt offline
);
CREATE INDEX games_filters ON games (username, time_class, end_time);

-- One row per decision the player made: the position they faced and what they played.
CREATE TABLE position_moves (
  game_id   TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
  ply       INT  NOT NULL,
  username  TEXT NOT NULL,                                 -- denormalized so lookups need no join
  epd       TEXT NOT NULL,                                 -- position key, see yvy_core::position_key
  uci       TEXT NOT NULL,
  san       TEXT NOT NULL,
  PRIMARY KEY (game_id, ply)
);
CREATE INDEX position_moves_lookup ON position_moves (username, epd);

-- A game played against the model.
CREATE TABLE sessions (
  id          UUID PRIMARY KEY,
  username    TEXT NOT NULL REFERENCES players(username) ON DELETE CASCADE,
  user_color  TEXT NOT NULL CHECK (user_color IN ('white', 'black')),
  engine_elo  INT  NOT NULL,
  mode        TEXT NOT NULL CHECK (mode IN ('sampled', 'most_frequent')),
  filters     JSONB NOT NULL DEFAULT '{}'::jsonb,
  status      TEXT NOT NULL DEFAULT 'playing',             -- playing | checkmate | stalemate | ... | resigned
  result      TEXT,                                        -- win | loss | draw, from the player's side
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE session_moves (
  session_id  UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  ply         INT  NOT NULL,
  uci         TEXT NOT NULL,
  san         TEXT NOT NULL,
  source      TEXT NOT NULL CHECK (source IN ('player', 'history', 'engine')),
  detail      JSONB,                                       -- candidate distribution or engine info
  PRIMARY KEY (session_id, ply)
);
