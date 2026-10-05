# Database Schema — Tempo 2

> **Status (2026-10-03): design problems known, not yet rewritten.** Any change below needs a
> migration in `tempo-project`.
>
> - **WAL mode** leaves `-wal` and `-shm` files next to the project, so copying or replacing
>   the `.tempo` file can lose data. The whole project is rewritten on save, so use the
>   default rollback journal instead.
> - **`thumbnail_data`** and **`waveform_cache`** should move out of the project file to
>   `~/.cache/tempo/`.
> - **`undo_history` / `undo_cursor`**: undo history does not need to survive closing the
>   project (it does not in Resolve either). Drop these tables.
> - **`clips.source_id NOT NULL`** and the duration `CHECK` block title clips and stills.
> - **Default track ids** such as `track-v1-uuid` are not valid UUIDs.
> - **`plugin_configs`**: plugin enabled state now lives in `~/.config/tempo/plugins.toml`.
>   Per-clip effect parameters need a place in the clip's `properties`.
> - **Markers**: colours are now blue, cyan, green, yellow, red, pink, purple
>   (`VISUAL_DESIGN.md §2.1`).
> - **Default tracks** are named `V1`, `V2`, `A1`, `A2`.
> - §16.8 computes duration from enabled tracks only, so hiding a track would shorten the
>   export. Duration should count every track.

**Version:** 1.0  
**Engine:** SQLite 3.40+ (bundled via rusqlite)  
**Mode:** WAL (Write-Ahead Logging) for concurrent reads during autosave  
**File extension:** `.tempo`  

A Tempo project is a single SQLite database file. This document defines the complete schema,
all table relationships, migration strategy, and example queries.

---

## 1. Design Decisions

- **Single file:** The entire project — timeline, clip metadata, settings, undo history — lives in
  one `.tempo` file. Source media files are never copied in; only their paths are stored.
- **WAL mode:** Enables reads (for render) during autosave writes (from background thread).
- **UUID primary keys:** All entities use UUID v4 stored as TEXT (36-char hyphenated). This
  ensures IDs remain globally unique across projects and don't collide when copy-pasting between
  projects in future versions.
- **Timestamps as INTEGER:** Stored as Unix timestamp in microseconds (i64). This avoids SQLite's
  text date ambiguities.
- **Rational FPS:** Stored as two INTEGER columns (fps_num, fps_den) to exactly represent 23.976,
  29.97, etc. without floating-point imprecision.
- **JSON for complex structures:** Clip properties (transform, color correction, keyframes) and
  plugin configs are stored as JSON blobs. This avoids schema churn as features are added.
- **No foreign key enforcement:** SQLite foreign keys are declared for documentation but disabled
  at runtime (application logic enforces referential integrity to avoid constraint errors during
  undo/redo).
- **Render Queue is Session-Only:** Export jobs and the render queue (`RenderQueue`, `ExportJob`)
  are maintained strictly in-memory within `tempo-export` during the active editor session and
  cleared on app quit. Active and queued export jobs are deliberately **not** persisted to the
  `.tempo` SQLite database file. This prevents stale file locks, broken output file handles upon
  relaunch, and unnecessary schema migrations. Export presets are stored in user preferences
  (`~/.config/tempo/`), not per-project database files.

---

## 2. Schema Initialization

```sql
-- Run on every open. Idempotent.
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = OFF;  -- enforced by application logic
PRAGMA synchronous = NORMAL; -- safe with WAL, faster than FULL
PRAGMA cache_size = -16000;  -- 16MB page cache
PRAGMA temp_store = MEMORY;
```

---

## 3. Schema Version Table

```sql
CREATE TABLE IF NOT EXISTS schema_version (
    version     INTEGER NOT NULL,
    applied_at  INTEGER NOT NULL,  -- Unix microseconds
    description TEXT
);

-- Initial version
INSERT INTO schema_version (version, applied_at, description)
VALUES (1, strftime('%s', 'now') * 1000000, 'Initial schema');
```

Current schema version: **1**

---

## 4. Project Metadata

```sql
CREATE TABLE project_meta (
    id            TEXT PRIMARY KEY,   -- UUID, single row
    name          TEXT NOT NULL,
    created_at    INTEGER NOT NULL,   -- Unix microseconds
    modified_at   INTEGER NOT NULL,
    -- Timeline settings
    width         INTEGER NOT NULL DEFAULT 1920,
    height        INTEGER NOT NULL DEFAULT 1080,
    fps_num       INTEGER NOT NULL DEFAULT 30,
    fps_den       INTEGER NOT NULL DEFAULT 1,
    sample_rate   INTEGER NOT NULL DEFAULT 48000,
    channels      INTEGER NOT NULL DEFAULT 2,
    -- Storage settings
    proxy_dir     TEXT NOT NULL,      -- absolute path
    -- UI state (restored on open)
    ui_state      TEXT NOT NULL DEFAULT '{}'  -- JSON blob
);
```

**ui_state JSON structure:**
```json
{
    "active_page": "cut",
    "timeline_zoom": 1.0,
    "timeline_scroll_x": 0,
    "left_panel_width": 300,
    "left_panel_tab": "media",
    "inspector_open": true,
    "viewer_split": 0.5
}
```

---

## 5. Media Sources

```sql
CREATE TABLE media_sources (
    id              TEXT PRIMARY KEY,       -- UUID
    path            TEXT NOT NULL,          -- absolute path at import time
    relative_path   TEXT NOT NULL,          -- relative to project file
    media_type      INTEGER NOT NULL,       -- 0=video 1=audio 2=image
    duration_us     INTEGER NOT NULL,       -- microseconds
    is_missing      INTEGER NOT NULL DEFAULT 0,  -- boolean

    -- Video stream info (NULL if audio-only)
    video_width     INTEGER,
    video_height    INTEGER,
    video_fps_num   INTEGER,
    video_fps_den   INTEGER,
    video_codec     TEXT,
    video_color_range  INTEGER,             -- 0=MPEG/limited 1=JPEG/full
    video_color_space  INTEGER,             -- 0=BT601 1=BT709 2=BT2020
    video_bit_depth    INTEGER,
    video_has_alpha    INTEGER DEFAULT 0,

    -- Audio stream info (NULL if video-only)
    audio_sample_rate  INTEGER,
    audio_channels     INTEGER,
    audio_codec        TEXT,
    audio_bit_rate     INTEGER,

    -- Proxy
    proxy_path      TEXT,                   -- NULL = no proxy generated
    proxy_ready     INTEGER NOT NULL DEFAULT 0,

    -- Organization
    imported_at     INTEGER NOT NULL,
    import_order    INTEGER NOT NULL DEFAULT 0,  -- for stable ordering in Media Pool
    thumbnail_data  BLOB                    -- 160×90 RGBA, stored inline for small overhead
);
```

---

## 6. Media Bins (Media Pool organization)

```sql
CREATE TABLE bins (
    id          TEXT PRIMARY KEY,    -- UUID
    name        TEXT NOT NULL,
    parent_id   TEXT,                -- NULL = root bin. FK → bins(id)
    sort_order  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE bin_sources (
    bin_id      TEXT NOT NULL,       -- FK → bins(id)
    source_id   TEXT NOT NULL,       -- FK → media_sources(id)
    sort_order  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (bin_id, source_id)
);

-- Root bin always exists
INSERT OR IGNORE INTO bins (id, name, parent_id, sort_order)
VALUES ('00000000-0000-0000-0000-000000000000', 'Master', NULL, 0);
```

---

## 7. Tracks

```sql
CREATE TABLE tracks (
    id          TEXT PRIMARY KEY,    -- UUID
    kind        INTEGER NOT NULL,    -- 0=video 1=audio
    kind_index  INTEGER NOT NULL,    -- 1-4 (V1/V2... or A1/A2...)
    name        TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,   -- boolean
    locked      INTEGER NOT NULL DEFAULT 0,   -- boolean
    solo        INTEGER NOT NULL DEFAULT 0,   -- boolean (audio only)
    volume      REAL    NOT NULL DEFAULT 1.0, -- audio only, 0.0-2.0
    height_px   INTEGER NOT NULL DEFAULT 72,  -- display height
    color       TEXT    NOT NULL DEFAULT '#5294e2',  -- hex color for UI
    sort_order  INTEGER NOT NULL               -- determines z-order and display order
);

-- Default tracks created with new project
INSERT INTO tracks (id, kind, kind_index, name, enabled, locked, sort_order) VALUES
    ('track-v1-uuid', 0, 1, 'Primary Video', 1, 0, 0),
    ('track-v2-uuid', 0, 2, 'B-Roll',        1, 0, 1),
    ('track-a1-uuid', 1, 1, 'Audio 1',       1, 0, 2),
    ('track-a2-uuid', 1, 2, 'Audio 2',       1, 0, 3);
```

---

## 8. Clips

```sql
CREATE TABLE clips (
    id              TEXT PRIMARY KEY,    -- UUID
    track_id        TEXT NOT NULL,       -- FK → tracks(id)
    source_id       TEXT NOT NULL,       -- FK → media_sources(id)
    clip_type       INTEGER NOT NULL,    -- 0=video 1=audio 2=image 3=title
    name            TEXT NOT NULL,

    -- Timeline position (microseconds)
    timeline_in     INTEGER NOT NULL,
    timeline_out    INTEGER NOT NULL,

    -- Source read range (microseconds within the source file)
    source_in       INTEGER NOT NULL,
    source_out      INTEGER NOT NULL,

    -- Properties stored as JSON (see §8.1)
    properties      TEXT NOT NULL DEFAULT '{}',

    -- Title data (NULL unless clip_type = 3)
    title_data      TEXT,   -- JSON, see §8.2

    -- Constraints
    CHECK (timeline_out > timeline_in),
    CHECK (source_out > source_in),
    CHECK ((source_out - source_in) = (timeline_out - timeline_in))
);

CREATE INDEX idx_clips_track ON clips (track_id, timeline_in);
CREATE INDEX idx_clips_source ON clips (source_id);
```

### 8.1 Clip Properties JSON

```json
{
    "opacity": 1.0,
    "position_x": 0.0,
    "position_y": 0.0,
    "scale_x": 1.0,
    "scale_y": 1.0,
    "rotation": 0.0,
    "volume": 1.0,
    "pan": 0.0,
    "muted": false,
    "lift": [0.0, 0.0, 0.0],
    "gamma": [1.0, 1.0, 1.0],
    "gain": [1.0, 1.0, 1.0],
    "keyframes": [
        {
            "time_us": 5000000,
            "property": "opacity",
            "value": 0.0
        },
        {
            "time_us": 8000000,
            "property": "opacity",
            "value": 1.0
        }
    ]
}
```

### 8.2 Title Data JSON

```json
{
    "title_type": "center_title",
    "text": "My Documentary",
    "subtitle": null,
    "font_family": "Sans",
    "font_size": 72.0,
    "font_bold": true,
    "font_italic": false,
    "color": [255, 255, 255, 255],
    "background_color": null,
    "background_padding": 16.0,
    "custom_position": null
}
```

---

## 9. Transitions

```sql
CREATE TABLE transitions (
    id              TEXT PRIMARY KEY,    -- UUID
    clip_id         TEXT NOT NULL,       -- FK → clips(id)
    edge            INTEGER NOT NULL,    -- 0=in 1=out
    kind            INTEGER NOT NULL,    -- 0=cut 1=cross_dissolve 2=dip_black 3=dip_white 4=fade_in 5=fade_out 6=plugin
    duration_us     INTEGER NOT NULL,
    alignment       INTEGER NOT NULL,    -- 0=centered 1=start_at_cut 2=end_at_cut
    plugin_id       TEXT,                -- NULL unless kind=6
    plugin_params   TEXT DEFAULT '{}',  -- JSON, plugin-specific parameters

    UNIQUE (clip_id, edge)
);
```

---

## 10. Markers

```sql
CREATE TABLE markers (
    id          TEXT PRIMARY KEY,    -- UUID
    position_us INTEGER NOT NULL,
    name        TEXT NOT NULL DEFAULT '',
    color       INTEGER NOT NULL DEFAULT 0,   -- 0=red 1=green 2=blue 3=yellow 4=orange 5=purple
    duration_us INTEGER NOT NULL DEFAULT 0,   -- 0=point marker
    note        TEXT NOT NULL DEFAULT ''
);

CREATE INDEX idx_markers_position ON markers (position_us);
```

---

## 11. Undo History

The command log is serialized to the database so undo persists across save/close/reopen.

```sql
CREATE TABLE undo_history (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    command_type    TEXT NOT NULL,       -- e.g., "InsertClip", "DeleteClip", "MoveClip"
    command_data    TEXT NOT NULL,       -- JSON, structure varies by command type
    description     TEXT NOT NULL,       -- human-readable ("Delete clip 'interview.mp4'")
    executed_at     INTEGER NOT NULL     -- Unix microseconds
);

-- cursor: position in history. Everything at seq > cursor is redo-able.
CREATE TABLE undo_cursor (
    id          INTEGER PRIMARY KEY DEFAULT 1,
    cursor_seq  INTEGER NOT NULL DEFAULT 0  -- seq of last executed command
);

INSERT OR IGNORE INTO undo_cursor (id, cursor_seq) VALUES (1, 0);
```

**Command data examples:**

InsertClip:
```json
{
    "clip_id": "...",
    "track_id": "...",
    "source_id": "...",
    "timeline_in": 5000000,
    "timeline_out": 15000000,
    "source_in": 0,
    "source_out": 10000000,
    "ripple": true
}
```

MoveClip:
```json
{
    "clip_id": "...",
    "from_track": "...",
    "to_track": "...",
    "old_timeline_in": 5000000,
    "new_timeline_in": 12000000
}
```

---

## 12. Plugin Configuration

```sql
CREATE TABLE plugin_configs (
    plugin_id   TEXT PRIMARY KEY,
    enabled     INTEGER NOT NULL DEFAULT 0,
    config      TEXT NOT NULL DEFAULT '{}'  -- JSON, plugin-specific settings
);
```

---

## 13. Waveform Cache Table

Waveform data is stored as a binary blob. Regenerated if source file mtime changes.

```sql
CREATE TABLE waveform_cache (
    source_id   TEXT PRIMARY KEY,    -- FK → media_sources(id)
    block_count INTEGER NOT NULL,
    data        BLOB NOT NULL,       -- packed f32 triplets: peak_pos, peak_neg, rms
    source_mtime INTEGER NOT NULL    -- source file mtime at generation time (Unix seconds)
);
```

---

## 14. App-Level Settings (stored in `~/.config/tempo/settings.toml`)

Settings are NOT stored in the project file — they are per-user, not per-project.
The project file only stores project-specific state.

See `docs/SYSTEM_DESIGN.md §9` for the `Settings` struct definition.

---

## 15. Migrations

### 15.1 Migration Runner

```rust
pub fn apply_migrations(conn: &rusqlite::Connection) -> Result<()> {
    let current = conn.query_row(
        "SELECT MAX(version) FROM schema_version",
        [],
        |row| row.get::<_, i64>(0),
    ).unwrap_or(0);

    for migration in MIGRATIONS.iter().filter(|m| m.version > current as u32) {
        conn.execute_batch(migration.sql)?;
        conn.execute(
            "INSERT INTO schema_version (version, applied_at, description) VALUES (?1, ?2, ?3)",
            rusqlite::params![migration.version, now_us(), migration.description],
        )?;
        tracing::info!("Applied migration {}: {}", migration.version, migration.description);
    }
    Ok(())
}

pub struct Migration {
    pub version: u32,
    pub description: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "Initial schema",
        sql: include_str!("migrations/001_initial.sql"),
    },
    // Future migrations added here:
    // Migration { version: 2, description: "Add subtitle tracks", sql: include_str!("...") },
];
```

### 15.2 Migration Policy

- Migrations are always additive in 1.x (add columns/tables, never remove).
- Removal requires a major version bump (2.0).
- Every migration runs inside a transaction — if it fails, the database is left unchanged.
- A project opened with a newer schema version than the running app version shows a warning:
  "This project was created with a newer version of Tempo. Some features may not work correctly."

---

## 16. Sample Queries

### 16.1 Load All Tracks in Display Order

```sql
SELECT id, kind, kind_index, name, enabled, locked, solo, volume, height_px, color
FROM tracks
ORDER BY sort_order ASC;
```

### 16.2 Load All Clips for a Track in Timeline Order

```sql
SELECT c.id, c.source_id, c.clip_type, c.name,
       c.timeline_in, c.timeline_out, c.source_in, c.source_out,
       c.properties, c.title_data,
       s.path, s.proxy_path, s.proxy_ready,
       s.video_width, s.video_height, s.video_fps_num, s.video_fps_den
FROM clips c
JOIN media_sources s ON c.source_id = s.id
WHERE c.track_id = ?1
ORDER BY c.timeline_in ASC;
```

### 16.3 Find All Clips at a Given Timeline Position

```sql
SELECT c.id, c.track_id, c.source_id, c.timeline_in, c.timeline_out,
       c.source_in, c.properties
FROM clips c
JOIN tracks t ON c.track_id = t.id
WHERE c.timeline_in <= ?1
  AND c.timeline_out > ?1
  AND t.enabled = 1
ORDER BY t.sort_order DESC;  -- DESC = top tracks first (highest z-order)
```

### 16.4 Find All Clips Using a Source File

```sql
SELECT c.id, c.track_id, c.name, c.timeline_in, c.timeline_out
FROM clips c
WHERE c.source_id = ?1
ORDER BY c.timeline_in ASC;
```

### 16.5 Load All Transitions for a Track's Clips

```sql
SELECT t.id, t.clip_id, t.edge, t.kind, t.duration_us, t.alignment, t.plugin_id, t.plugin_params
FROM transitions t
JOIN clips c ON t.clip_id = c.id
WHERE c.track_id = ?1
ORDER BY c.timeline_in ASC, t.edge ASC;
```

### 16.6 Load Markers in Range

```sql
SELECT id, position_us, name, color, duration_us, note
FROM markers
WHERE position_us BETWEEN ?1 AND ?2
ORDER BY position_us ASC;
```

### 16.7 Load Undo History (for restoring undo/redo state)

```sql
SELECT seq, command_type, command_data, description, executed_at
FROM undo_history
ORDER BY seq ASC;

SELECT cursor_seq FROM undo_cursor WHERE id = 1;
```

### 16.8 Compute Timeline Duration

```sql
SELECT MAX(timeline_out) AS duration_us
FROM clips c
JOIN tracks t ON c.track_id = t.id
WHERE t.enabled = 1;
```

### 16.9 Check for Missing Media

```sql
SELECT id, path, relative_path, media_type
FROM media_sources
WHERE is_missing = 1;
```

### 16.10 Get Waveform Data

```sql
SELECT data, block_count FROM waveform_cache WHERE source_id = ?1;
```

---

## 17. Full Schema DDL (reference)

```sql
-- Apply all at once for new project creation:

PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = OFF;
PRAGMA synchronous = NORMAL;

CREATE TABLE schema_version (
    version     INTEGER NOT NULL,
    applied_at  INTEGER NOT NULL,
    description TEXT
);

CREATE TABLE project_meta (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    modified_at INTEGER NOT NULL,
    width       INTEGER NOT NULL DEFAULT 1920,
    height      INTEGER NOT NULL DEFAULT 1080,
    fps_num     INTEGER NOT NULL DEFAULT 30,
    fps_den     INTEGER NOT NULL DEFAULT 1,
    sample_rate INTEGER NOT NULL DEFAULT 48000,
    channels    INTEGER NOT NULL DEFAULT 2,
    proxy_dir   TEXT NOT NULL,
    ui_state    TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE media_sources (
    id                TEXT PRIMARY KEY,
    path              TEXT NOT NULL,
    relative_path     TEXT NOT NULL,
    media_type        INTEGER NOT NULL,
    duration_us       INTEGER NOT NULL,
    is_missing        INTEGER NOT NULL DEFAULT 0,
    video_width       INTEGER,
    video_height      INTEGER,
    video_fps_num     INTEGER,
    video_fps_den     INTEGER,
    video_codec       TEXT,
    video_color_range INTEGER,
    video_color_space INTEGER,
    video_bit_depth   INTEGER,
    video_has_alpha   INTEGER DEFAULT 0,
    audio_sample_rate INTEGER,
    audio_channels    INTEGER,
    audio_codec       TEXT,
    audio_bit_rate    INTEGER,
    proxy_path        TEXT,
    proxy_ready       INTEGER NOT NULL DEFAULT 0,
    imported_at       INTEGER NOT NULL,
    import_order      INTEGER NOT NULL DEFAULT 0,
    thumbnail_data    BLOB
);

CREATE TABLE bins (
    id        TEXT PRIMARY KEY,
    name      TEXT NOT NULL,
    parent_id TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE bin_sources (
    bin_id    TEXT NOT NULL,
    source_id TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (bin_id, source_id)
);

CREATE TABLE tracks (
    id         TEXT PRIMARY KEY,
    kind       INTEGER NOT NULL,
    kind_index INTEGER NOT NULL,
    name       TEXT NOT NULL,
    enabled    INTEGER NOT NULL DEFAULT 1,
    locked     INTEGER NOT NULL DEFAULT 0,
    solo       INTEGER NOT NULL DEFAULT 0,
    volume     REAL    NOT NULL DEFAULT 1.0,
    height_px  INTEGER NOT NULL DEFAULT 72,
    color      TEXT    NOT NULL DEFAULT '#5294e2',
    sort_order INTEGER NOT NULL
);

CREATE TABLE clips (
    id           TEXT PRIMARY KEY,
    track_id     TEXT NOT NULL,
    source_id    TEXT NOT NULL,
    clip_type    INTEGER NOT NULL,
    name         TEXT NOT NULL,
    timeline_in  INTEGER NOT NULL,
    timeline_out INTEGER NOT NULL,
    source_in    INTEGER NOT NULL,
    source_out   INTEGER NOT NULL,
    properties   TEXT NOT NULL DEFAULT '{}',
    title_data   TEXT,
    CHECK (timeline_out > timeline_in),
    CHECK (source_out > source_in)
);

CREATE INDEX idx_clips_track ON clips (track_id, timeline_in);
CREATE INDEX idx_clips_source ON clips (source_id);

CREATE TABLE transitions (
    id            TEXT PRIMARY KEY,
    clip_id       TEXT NOT NULL,
    edge          INTEGER NOT NULL,
    kind          INTEGER NOT NULL,
    duration_us   INTEGER NOT NULL,
    alignment     INTEGER NOT NULL,
    plugin_id     TEXT,
    plugin_params TEXT DEFAULT '{}',
    UNIQUE (clip_id, edge)
);

CREATE TABLE markers (
    id          TEXT PRIMARY KEY,
    position_us INTEGER NOT NULL,
    name        TEXT NOT NULL DEFAULT '',
    color       INTEGER NOT NULL DEFAULT 0,
    duration_us INTEGER NOT NULL DEFAULT 0,
    note        TEXT NOT NULL DEFAULT ''
);

CREATE INDEX idx_markers_position ON markers (position_us);

CREATE TABLE undo_history (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    command_type TEXT NOT NULL,
    command_data TEXT NOT NULL,
    description  TEXT NOT NULL,
    executed_at  INTEGER NOT NULL
);

CREATE TABLE undo_cursor (
    id         INTEGER PRIMARY KEY DEFAULT 1,
    cursor_seq INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE plugin_configs (
    plugin_id TEXT PRIMARY KEY,
    enabled   INTEGER NOT NULL DEFAULT 0,
    config    TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE waveform_cache (
    source_id    TEXT PRIMARY KEY,
    block_count  INTEGER NOT NULL,
    data         BLOB NOT NULL,
    source_mtime INTEGER NOT NULL
);

-- Seed data
INSERT INTO schema_version VALUES (1, strftime('%s','now') * 1000000, 'Initial schema');
INSERT INTO undo_cursor (id, cursor_seq) VALUES (1, 0);
INSERT INTO bins (id, name, parent_id, sort_order)
VALUES ('00000000-0000-0000-0000-000000000000', 'Master', NULL, 0);
```

---

## 11. Render Queue & Export State (Session-Only Architecture)

> **AGENT NOTE:** Do NOT add an `export_jobs` or `render_queue` table to this schema.

The Export Page (`[Cut] | [Edit] | [Export]`) and its Render Queue operate with an **in-memory, session-only architecture**:

1. **State Ownership:** The `RenderQueue` struct lives in `crates/tempo-export` and is managed in memory by `tempo-ui`'s session controller.
2. **Session Lifecycle:** When the user closes Tempo, any active, queued, or completed jobs are discarded from memory.
3. **Rationale:**
   - A video export job references transient file handles and ephemeral encoder state. Persisting incomplete encodes across reboots leads to orphan files, invalid GPU contexts, and stale progress counters.
   - Preserves SQLite database portability: moving a `.tempo` file to another computer does not carry broken absolute output paths.
4. **User Presets:** Export preset configurations (e.g. custom bitrates, platform resolutions) are saved to application-level JSON configuration (`~/.config/tempo/export_presets.json`), never in the project SQLite database.
