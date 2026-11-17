-- Schema version 9: playlists of media jobs (docs/VIDEO.md, "Playlists: as
-- built"). Each selected entry of a playlist is a media job of its own; the
-- playlist groups them, names their folder and runs them one at a time.
-- Times are Unix milliseconds.

CREATE TABLE media_playlist (
  id         INTEGER PRIMARY KEY AUTOINCREMENT, -- never reused, like job ids
  page_url   TEXT NOT NULL,                     -- the playlist's page, as given
  title      TEXT,                              -- what the probe found
  uploader   TEXT,
  dest_dir   TEXT NOT NULL,                     -- the playlist's folder (its entries' yt-dlp -P)
  created_at INTEGER NOT NULL
);

ALTER TABLE media_job ADD COLUMN playlist_id INTEGER REFERENCES media_playlist(id) ON DELETE SET NULL; -- NULL: a single page
ALTER TABLE media_job ADD COLUMN playlist_index INTEGER; -- the entry's place in the playlist, from 1

CREATE INDEX media_job_playlist_idx ON media_job(playlist_id);
