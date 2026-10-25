-- Schema version 2: per-download speed limits. The global limit is a row in
-- `setting` (key 'engine.speed_limit_bps').
ALTER TABLE download ADD COLUMN speed_limit_bps INTEGER; -- bytes per second; NULL = no per-download limit
