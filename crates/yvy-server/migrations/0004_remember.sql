-- Whether the human's moves in this session are added to their history (as a game with
-- time class 'yvy'), so the model learns from games played here, not only from chess.com.
ALTER TABLE sessions ADD COLUMN remember BOOLEAN NOT NULL DEFAULT true;
