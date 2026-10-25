-- Schema version 1: downloads, their segment checkpoints, and settings.
-- See docs/ARCHITECTURE.md, "Data model". Sizes and offsets are bytes, times
-- are Unix milliseconds.

CREATE TABLE download (
  id              INTEGER PRIMARY KEY AUTOINCREMENT, -- never reused, shown by the CLI
  kind            TEXT NOT NULL DEFAULT 'http',
  state           TEXT NOT NULL,                     -- queued | probing | downloading | retrying | paused | completed | failed | cancelled
  url             TEXT NOT NULL,                     -- as given by the user
  final_url       TEXT,                              -- after redirects, last successful
  request_ctx     TEXT NOT NULL,                     -- JSON: non-secret headers, suggested name
  requested_kind  TEXT NOT NULL,                     -- 'dir' | 'file': how the destination was given
  requested_dest  TEXT NOT NULL,                     -- that directory or file path
  conflict        TEXT NOT NULL,                     -- 'rename' | 'overwrite' | 'fail'
  file_name       TEXT,                              -- resolved name; NULL until probed
  dest_dir        TEXT,                              -- resolved directory; NULL until probed
  temp_path       TEXT,                              -- '<name>.boltget' while in progress
  total_size      INTEGER,                           -- NULL if unknown
  downloaded      INTEGER NOT NULL DEFAULT 0,        -- durable bytes (sum of segment progress)
  mime            TEXT,
  etag            TEXT,                              -- as received, including W/ prefix
  last_modified   TEXT,                              -- HTTP-date as received
  validator_date  TEXT,                              -- Date of the probe response (If-Range rule)
  resumable       INTEGER NOT NULL DEFAULT 0,        -- byte ranges confirmed
  encoded         INTEGER NOT NULL DEFAULT 0,        -- server applied a Content-Encoding
  max_connections INTEGER,                           -- NULL = engine default
  error_kind      TEXT,
  error_key       TEXT,
  error_detail    TEXT,
  created_at      INTEGER NOT NULL,
  started_at      INTEGER,
  completed_at    INTEGER,
  updated_at      INTEGER NOT NULL
);

CREATE TABLE segment (
  download_id  INTEGER NOT NULL REFERENCES download(id) ON DELETE CASCADE,
  idx          INTEGER NOT NULL,      -- identifier within the download
  start        INTEGER NOT NULL,      -- inclusive byte offset
  "end"        INTEGER,               -- exclusive byte offset; NULL while the size is unknown
  done         INTEGER NOT NULL,      -- durable bytes from start (start + done = cursor)
  PRIMARY KEY (download_id, idx)
) WITHOUT ROWID;

CREATE TABLE setting (
  key    TEXT PRIMARY KEY,
  value  TEXT NOT NULL               -- JSON
);

CREATE INDEX download_state_idx ON download(state);
