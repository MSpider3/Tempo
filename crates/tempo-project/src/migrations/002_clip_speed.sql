-- Clip speed (0.2.0): a retimed clip covers more or less source than timeline, so the
-- rule that the two lengths are equal goes. SQLite can't drop a CHECK in place, so the
-- table is rebuilt with the same columns and the other two checks.
CREATE TABLE clips_new (
    id              TEXT PRIMARY KEY,
    track_id        TEXT NOT NULL,
    source_id       TEXT NOT NULL,
    clip_type       INTEGER NOT NULL,
    name            TEXT NOT NULL,
    timeline_in     INTEGER NOT NULL,
    timeline_out    INTEGER NOT NULL,
    source_in       INTEGER NOT NULL,
    source_out      INTEGER NOT NULL,
    properties      TEXT NOT NULL DEFAULT '{}',
    title_data      TEXT,
    CHECK (timeline_out > timeline_in),
    CHECK (source_out > source_in)
);

INSERT INTO clips_new (id, track_id, source_id, clip_type, name, timeline_in, timeline_out, source_in, source_out, properties, title_data)
SELECT id, track_id, source_id, clip_type, name, timeline_in, timeline_out, source_in, source_out, properties, title_data FROM clips;

DROP TABLE clips;
ALTER TABLE clips_new RENAME TO clips;

CREATE INDEX IF NOT EXISTS idx_clips_track ON clips (track_id, timeline_in);
CREATE INDEX IF NOT EXISTS idx_clips_source ON clips (source_id);
