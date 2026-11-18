-- Schema version 10: the name stem of a media job's files (docs/VIDEO.md,
-- "Errors and pause/resume: as built"). yt-dlp names every file of a video
-- after the same stem ("Title [id]": "Title [id].f137.mp4.part", ...); kept
-- once yt-dlp reports its first file, so a job removed after a restart can
-- still find and delete its partial files in its folder.

ALTER TABLE media_job ADD COLUMN file_stem TEXT; -- NULL until yt-dlp names a file
