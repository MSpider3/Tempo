-- Several timelines per project (0.3.0), as in Resolve. Tracks and markers say which
-- timeline they belong to; clips and transitions follow their track and clip. Rows from
-- older files have no timeline and load into the open one.
CREATE TABLE IF NOT EXISTS timelines (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    is_open     INTEGER NOT NULL DEFAULT 0
);

ALTER TABLE tracks ADD COLUMN timeline_id TEXT;
ALTER TABLE markers ADD COLUMN timeline_id TEXT;
