-- Resolve parity (0.5.0). Timelines: Start Timecode, and their own width and height when
-- "Use Project Settings" was off (NULL = the project's). Tracks: Audio Track Type
-- (0 = mono, 1 = stereo). Markers: Keyword. Transitions: Start Ratio and End Ratio
-- (0.0-1.0) and the Transition Curve keys as JSON '[[place, value], ...]' (NULL = none).
ALTER TABLE timelines ADD COLUMN start_tc_us INTEGER NOT NULL DEFAULT 0;
ALTER TABLE timelines ADD COLUMN width INTEGER;
ALTER TABLE timelines ADD COLUMN height INTEGER;
ALTER TABLE tracks ADD COLUMN audio_type INTEGER NOT NULL DEFAULT 1;
ALTER TABLE markers ADD COLUMN keyword TEXT NOT NULL DEFAULT '';
ALTER TABLE transitions ADD COLUMN start_ratio REAL NOT NULL DEFAULT 0;
ALTER TABLE transitions ADD COLUMN end_ratio REAL NOT NULL DEFAULT 1;
ALTER TABLE transitions ADD COLUMN curve TEXT;
