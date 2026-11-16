-- Schema version 6: downloads from a private browser window
-- (docs/BROWSER-INTEGRATION.md, "What is never captured"). A front end may
-- forget such a download (delete its row, never its file) once it completes.
ALTER TABLE download ADD COLUMN private INTEGER NOT NULL DEFAULT 0;
