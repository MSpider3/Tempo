-- Tempo 2 Initial Schema (v1)

CREATE TABLE IF NOT EXISTS schema_version (
    version     INTEGER NOT NULL,
    applied_at  INTEGER NOT NULL,
    description TEXT
);

CREATE TABLE IF NOT EXISTS project_meta (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    modified_at   INTEGER NOT NULL,
    width         INTEGER NOT NULL DEFAULT 1920,
    height        INTEGER NOT NULL DEFAULT 1080,
    fps_num       INTEGER NOT NULL DEFAULT 30,
    fps_den       INTEGER NOT NULL DEFAULT 1,
    sample_rate   INTEGER NOT NULL DEFAULT 48000,
    channels      INTEGER NOT NULL DEFAULT 2,
    proxy_dir     TEXT NOT NULL,
    ui_state      TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS media_sources (
    id              TEXT PRIMARY KEY,
    path            TEXT NOT NULL,
    relative_path   TEXT NOT NULL,
    media_type      INTEGER NOT NULL,
    duration_us     INTEGER NOT NULL,
    is_missing      INTEGER NOT NULL DEFAULT 0,
    video_width     INTEGER,
    video_height    INTEGER,
    video_fps_num   INTEGER,
    video_fps_den   INTEGER,
    video_codec     TEXT,
    video_color_range  INTEGER,
    video_color_space  INTEGER,
    video_bit_depth    INTEGER,
    video_has_alpha    INTEGER DEFAULT 0,
    audio_sample_rate  INTEGER,
    audio_channels     INTEGER,
    audio_codec        TEXT,
    audio_bit_rate     INTEGER,
    proxy_path      TEXT,
    proxy_ready     INTEGER NOT NULL DEFAULT 0,
    imported_at     INTEGER NOT NULL,
    import_order    INTEGER NOT NULL DEFAULT 0,
    thumbnail_data  BLOB
);

CREATE TABLE IF NOT EXISTS bins (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    parent_id   TEXT,
    sort_order  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS bin_sources (
    bin_id      TEXT NOT NULL,
    source_id   TEXT NOT NULL,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (bin_id, source_id)
);

CREATE TABLE IF NOT EXISTS tracks (
    id          TEXT PRIMARY KEY,
    kind        INTEGER NOT NULL,
    kind_index  INTEGER NOT NULL,
    name        TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    locked      INTEGER NOT NULL DEFAULT 0,
    solo        INTEGER NOT NULL DEFAULT 0,
    volume      REAL    NOT NULL DEFAULT 1.0,
    height_px   INTEGER NOT NULL DEFAULT 72,
    color       TEXT    NOT NULL DEFAULT '#5294e2',
    sort_order  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS clips (
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
    CHECK (source_out > source_in),
    CHECK ((source_out - source_in) = (timeline_out - timeline_in))
);

CREATE INDEX IF NOT EXISTS idx_clips_track ON clips (track_id, timeline_in);
CREATE INDEX IF NOT EXISTS idx_clips_source ON clips (source_id);

CREATE TABLE IF NOT EXISTS transitions (
    id              TEXT PRIMARY KEY,
    clip_id         TEXT NOT NULL,
    edge            INTEGER NOT NULL,
    kind            INTEGER NOT NULL,
    duration_us     INTEGER NOT NULL,
    alignment       INTEGER NOT NULL,
    plugin_id       TEXT,
    plugin_params   TEXT DEFAULT '{}',
    UNIQUE (clip_id, edge)
);

CREATE TABLE IF NOT EXISTS markers (
    id          TEXT PRIMARY KEY,
    position_us INTEGER NOT NULL,
    name        TEXT NOT NULL DEFAULT '',
    color       INTEGER NOT NULL DEFAULT 0,
    duration_us INTEGER NOT NULL DEFAULT 0,
    note        TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_markers_position ON markers (position_us);

CREATE TABLE IF NOT EXISTS undo_history (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    command_type    TEXT NOT NULL,
    command_data    TEXT NOT NULL,
    description     TEXT NOT NULL,
    executed_at     INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS undo_cursor (
    id          INTEGER PRIMARY KEY DEFAULT 1,
    cursor_seq  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS plugin_configs (
    plugin_id   TEXT PRIMARY KEY,
    enabled     INTEGER NOT NULL DEFAULT 0,
    config      TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS waveform_cache (
    source_id       TEXT PRIMARY KEY,
    block_count     INTEGER NOT NULL,
    data            BLOB NOT NULL,
    source_mtime    INTEGER NOT NULL
);

-- Master root bin
INSERT OR IGNORE INTO bins (id, name, parent_id, sort_order)
VALUES ('00000000-0000-0000-0000-000000000000', 'Master', NULL, 0);

-- Initial undo cursor
INSERT OR IGNORE INTO undo_cursor (id, cursor_seq) VALUES (1, 0);
