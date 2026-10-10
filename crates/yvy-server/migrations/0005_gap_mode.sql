-- What happens when the model has no history for the position it must move in:
--   stockfish: the engine fills the gap (with steering), as before;
--   switch:    the human takes over the model's side and plays the position themselves.
ALTER TABLE sessions ADD COLUMN gap_mode TEXT NOT NULL DEFAULT 'stockfish' CHECK (gap_mode IN ('stockfish', 'switch'));
-- The side the human plays right now (changes on a switch); user_color keeps the starting side.
ALTER TABLE sessions ADD COLUMN current_color TEXT;
UPDATE sessions SET current_color = user_color;
ALTER TABLE sessions ALTER COLUMN current_color SET NOT NULL;
-- Switch points: [{"ply": 6, "color": "black"}] means that after 6 moves the human took over Black.
ALTER TABLE sessions ADD COLUMN switches JSONB NOT NULL DEFAULT '[]'::jsonb;
