-- Schema version 8: where a media job's file ended up, and its size
-- (docs/VIDEO.md, "Picker and media downloads: as built"). yt-dlp names the
-- file itself, so the job learns its path only when it completes.

ALTER TABLE media_job ADD COLUMN file_path TEXT;  -- the finished file; NULL until completed
ALTER TABLE media_job ADD COLUMN size INTEGER;    -- bytes: the picker's estimate, then the file's size
