-- Schema version 4: link refresh history. `download.url` is the link
-- re-resolution starts from; after a refresh it is the new link, and every
-- link the download ever used is listed here.
CREATE TABLE download_source (
  download_id  INTEGER NOT NULL REFERENCES download(id) ON DELETE CASCADE,
  url          TEXT NOT NULL,
  role         TEXT NOT NULL,   -- 'primary' (as added) | 'refreshed' (given later)
  added_at     INTEGER NOT NULL,
  last_error   TEXT,
  PRIMARY KEY (download_id, url)
) WITHOUT ROWID;
