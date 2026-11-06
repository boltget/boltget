-- Schema version 5: queues (docs/ARCHITECTURE.md, "Data model"; docs/ENGINE.md,
-- "Queues"). Every download belongs to a queue; the default queue ("Main",
-- id 1) cannot be deleted. Times are Unix milliseconds; daily times are
-- minutes after local midnight.

CREATE TABLE queue (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  name            TEXT,                         -- NULL for the default queue: front ends name it ("Main")
  is_default      INTEGER NOT NULL DEFAULT 0,
  sort_order      INTEGER NOT NULL,             -- order of the queues in a front end
  max_concurrent  INTEGER NOT NULL DEFAULT 3,   -- downloads it runs at once
  started         INTEGER NOT NULL DEFAULT 1,   -- 1: starts waiting downloads as slots free
  start_minute    INTEGER,                      -- daily: start the queue at this local time
  stop_minute     INTEGER,                      -- daily: stop it at this local time
  window_applied  INTEGER,                      -- the last daily start or stop acted on (Unix ms)
  on_finish       TEXT NOT NULL DEFAULT 'nothing' -- 'nothing' | 'sleep' | 'shut_down'
);

INSERT INTO queue (id, name, is_default, sort_order) VALUES (1, NULL, 1, 0);

-- A column with a REFERENCES clause must default to NULL; NULL reads as the
-- default queue, and every insert sets it.
ALTER TABLE download ADD COLUMN queue_id INTEGER REFERENCES queue(id) ON DELETE SET NULL;
ALTER TABLE download ADD COLUMN queue_position INTEGER NOT NULL DEFAULT 0;
ALTER TABLE download ADD COLUMN start_at INTEGER;  -- start at this time (Unix ms), whatever the queue does

UPDATE download SET queue_id = 1, queue_position = id;

CREATE INDEX download_queue_idx ON download(queue_id, queue_position);
