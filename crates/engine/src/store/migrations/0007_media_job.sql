-- Schema version 7: media jobs, downloads of video or audio from a web page
-- that yt-dlp carries out (docs/VIDEO.md; docs/ARCHITECTURE.md, "Data
-- model"). Times are Unix milliseconds.

CREATE TABLE media_job (
  id                INTEGER PRIMARY KEY AUTOINCREMENT, -- never reused, like download ids
  download_id       INTEGER UNIQUE REFERENCES download(id) ON DELETE CASCADE, -- the row it shows as; NULL until it has one
  state             TEXT NOT NULL,                     -- queued | downloading | processing | paused | completed | failed | cancelled
  page_url          TEXT NOT NULL,                     -- the web page, as given
  title             TEXT,                              -- what the probe found
  format_expression TEXT NOT NULL,                     -- yt-dlp -f
  preset            TEXT,                              -- best | 1080p | 720p | small | audio; NULL: typed by the user
  container         TEXT NOT NULL,                     -- mp4 (Prefer MP4) | original
  subtitles         TEXT,                              -- JSON {languages, automatic, embed}; NULL: none
  output_template   TEXT NOT NULL,                     -- yt-dlp -o
  dest_dir          TEXT NOT NULL,                     -- the folder (yt-dlp -P)
  error_key         TEXT,                              -- message-catalog key, when failed
  error_detail      TEXT,                              -- yt-dlp's message, for example
  created_at        INTEGER NOT NULL,
  updated_at        INTEGER NOT NULL
);

CREATE INDEX media_job_state_idx ON media_job(state);
