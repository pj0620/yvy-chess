-- How far (in centipawns) a gap-filling move may fall below Stockfish's best move in order to
-- head back toward positions the player has faced. 0 turns steering off.
ALTER TABLE sessions ADD COLUMN steer_cp INT NOT NULL DEFAULT 100;
