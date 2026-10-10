-- Placement of the player's own pieces after each move, for setup-based steering.
-- Rows from older imports have NULL here until `yvy-server reindex` (run automatically at
-- startup for affected players) rebuilds them from the stored PGNs.
ALTER TABLE position_moves ADD COLUMN setup TEXT;
CREATE INDEX position_moves_setup ON position_moves (username, setup);
