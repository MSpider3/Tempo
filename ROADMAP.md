# Tempo — Development Roadmap

> **Tempo** — Lightweight, CPU-only video editor inspired by DaVinci Resolve's Cut Page.
>
> This roadmap outlines a week-by-week development plan across 4 phases,
> from project skeleton to production-ready desktop application.

---

## Overview

| Phase | Weeks | Goal |
|-------|-------|------|
| **Phase 1** — Core Foundation | Weeks 1–4 | A working skeleton you can actually run |
| **Phase 2** — Timeline + Editing | Weeks 5–9 | Actually edit something |
| **Phase 3** — Transitions + Speed + Text + Export | Weeks 10–14 | Produce an actual output file |
| **Phase 4** — Polish + Performance | Weeks 15+ | Feels like a real app |

---

## Phase 1 — Core Foundation (Weeks 1–4)

**Goal:** A working skeleton you can actually run. Import a video, see it in the
media bin, and play it in the preview player.

### Week 1: Project Setup & Core Data Models

**Deliverables:**
- [ ] Initialize project with `uv init`, configure `pyproject.toml`
- [ ] Set up `ruff.toml` (linting) and `mypy.ini` (type checking, strict mode)
- [ ] Create `.gitignore`, `.python-version` (3.11)
- [ ] Create complete directory structure (all `__init__.py` files, empty modules)
- [ ] Implement core data models in `core/models.py`:
  - `MediaItem` — imported file metadata
  - `Clip` — timeline clip (video/audio) with in/out, speed, transitions
  - `Track` — ordered list of clips with type (video/audio)
  - `TextClip` — text overlay on TX track
  - `TransitionConfig` — transition type and duration
  - `ProjectSettings` — resolution, framerate, sample rate
  - `Project` — top-level container with media list and timeline
- [ ] Implement project I/O in `core/project.py`:
  - `save_project(project, path)` → JSON serialization
  - `load_project(path)` → JSON deserialization with validation
  - Versioned `.tempo` file format (version "1.0")
- [ ] Write unit tests: `tests/test_models.py`, `tests/test_project.py`
- [ ] Implement `utils/timecode.py` — time formatting helpers
- [ ] Implement `utils/logger.py` — app-wide logging setup

**Dependencies:** None (first week).

**Exit criteria:** `pytest` passes. Can create a Project dataclass, serialize to
JSON, and deserialize back identically.

---

### Week 2: FFmpeg Integration & Media Pipeline

**Deliverables:**
- [ ] Implement `utils/ffmpeg.py` — FFmpeg/ffprobe subprocess wrapper:
  - `probe(path)` → extract metadata (duration, resolution, codec, fps, audio info)
  - `generate_proxy(input_path, output_path, target_height)` → re-encode at lower res
  - `extract_thumbnail(input_path, output_path, timestamp)` → single frame JPEG
  - `extract_waveform_peaks(input_path)` → audio peak data for visualization
  - Error handling: parse stderr, timeout support, missing FFmpeg detection
- [ ] Implement `utils/cache.py` — proxy and thumbnail cache manager:
  - Cache location: `~/.tempo/proxies/`, `~/.tempo/thumbs/`
  - Cache key: SHA-256 hash of `(absolute_path + file_size + mtime)`
  - Lookup, create, invalidate, and cleanup operations
- [ ] Implement `media/proxy.py` — proxy generation logic:
  - Determine target resolution: 480p for 1080p+ source, 360p for 4K+ source
  - Fast preset, CRF 28, H.264 baseline profile for maximum compatibility
  - Skip proxy if source is already ≤ 480p
- [ ] Implement `media/thumbnail.py` — thumbnail extraction:
  - Extract frame at 10% of duration (or 1 second, whichever is less)
  - Output as 160×90 JPEG
- [ ] Implement `media/waveform.py` — waveform data extraction:
  - Extract peak data at ~100 samples per second
  - Return as list of (min, max) float tuples
- [ ] Implement `media/importer.py` — media import pipeline:
  - Validate file type (check extension against allowed formats)
  - Call ffprobe to extract metadata
  - Create `MediaItem` dataclass
  - Queue proxy generation and thumbnail extraction
- [ ] Write unit tests: `tests/test_ffmpeg.py` (requires FFmpeg installed)

**Dependencies:** Week 1 (core models).

**Exit criteria:** Can probe a video file, generate a proxy, extract a thumbnail,
and create a `MediaItem` with all metadata populated.

---

### Week 3: UI Shell & Dark Theme

**Deliverables:**
- [ ] Implement `app.py` — QApplication setup with app name, version, org
- [ ] Implement `__main__.py` — entry point (`python -m tempo`)
- [ ] Implement `ui/theme.py` — complete design system:
  - All color constants (background, panel, surface, border, accent, tracks, etc.)
  - Typography (Inter font family, JetBrains Mono for timecodes)
  - QStyleSheet generation for dark theme
  - Utility functions: `apply_theme(app)`, `get_color(name)`, `get_font(name)`
- [ ] Implement `ui/main_window.py` — QMainWindow:
  - Three-panel layout using QSplitter (horizontal: media bin | preview | inspector)
  - Timeline area below panels (vertical splitter)
  - Menu bar: File, Edit, View, Export, Help
  - Menu items with shortcuts (Ctrl+I, Ctrl+S, Ctrl+O, Ctrl+E, etc.)
  - Status bar with project name and timeline duration
- [ ] Implement `ui/panels/media_bin.py` — left panel:
  - QListWidget with custom delegate for thumbnails
  - Each item shows: thumbnail (80×45), filename, duration, file type icon
  - [+ Import] button at bottom
  - Import triggers file dialog (Ctrl+I shortcut)
  - Items show status: importing (spinner), ready, missing (red icon)
- [ ] Stub `ui/panels/preview.py` — placeholder widget
- [ ] Stub `ui/panels/inspector.py` — placeholder widget
- [ ] Stub `ui/timeline/timeline_widget.py` — placeholder widget

**Dependencies:** Week 1 (core models), Week 2 (importer for file import).

**Exit criteria:** App launches with dark theme. Media bin panel visible on the
left. Can click Import, select a file, see it appear in the media bin with
thumbnail and duration.

---

### Week 4: Preview Player

**Deliverables:**
- [ ] Implement `ui/panels/preview.py` — MPV-based preview player:
  - Embed MPV player in a QWidget container
  - `python-mpv` integration with proper Qt event loop handling
  - Load and play proxy files (not originals)
  - Play / Pause toggle (Space key)
  - Step forward/back one frame (← → keys)
  - Step forward/back one second (Shift+← Shift+→)
  - Go to start / end (Home / End keys)
  - Current timecode display (HH:MM:SS:FF format)
  - Total duration display
  - Seek by clicking on a progress bar
- [ ] Transport controls widget:
  - Buttons: |◄ ◄◄ ►/║ ►► ►| (go start, step back, play/pause, step forward, go end)
  - Timecode readout: `00:01:23 / 00:05:00`
- [ ] Connect media bin to preview:
  - Double-click on media bin item → load in preview player
  - Preview plays the proxy file, with proxy path from `MediaItem`
- [ ] In/Out point system:
  - `I` key marks In point at current playhead
  - `O` key marks Out point at current playhead
  - `Alt+I` / `Alt+O` clears In / Out
  - Visual indicators on the preview progress bar
- [ ] Basic J/K/L shuttle control:
  - State machine: `J` = rewind (stack speed), `K` = stop, `L` = forward (stack speed)
  - Speed levels: 1×, 2×, 4×, 8×

**Dependencies:** Week 2 (proxy files), Week 3 (UI shell).

**Exit criteria:** Can import a video, see it in the media bin, double-click it,
and play it in the preview player with full transport controls. J/K/L shuttle
works. In/Out points can be set.

---

### Phase 1 Exit Criteria ✅

> **Can import a video, see it in the bin, and click play in the preview.**
>
> The app launches, shows a dark professional theme, imports media with
> background proxy generation, and plays video in an embedded MPV player.

---

## Phase 2 — Timeline + Editing (Weeks 5–9)

**Goal:** Actually edit something. Build a multi-clip, multi-track timeline with
full editing operations, preview integration, and all keybindings.

### Week 5: Timeline Foundation

**Deliverables:**
- [ ] Implement `ui/timeline/timeline_widget.py` — outer container:
  - Vertical layout: upper timeline (overview) + lower timeline (detail)
  - Resizable split between upper and lower
  - Toolbar below: Select (A), Blade (B), Trim (T), Text (Ctrl+T), Zoom slider
- [ ] Implement `ui/timeline/lower_timeline.py` — detail timeline:
  - QGraphicsScene + QGraphicsView
  - Coordinate system: X-axis = time (configurable pixels-per-second), Y-axis = tracks
  - Track lanes for: TX, V3, V2, V1, A1, A2, A3
  - Time ruler at top (ticks at 1s, 5s, 10s, 30s, 1m intervals depending on zoom)
  - Horizontal scrollbar for panning
- [ ] Implement `ui/timeline/track_header.py` — track labels:
  - Vertical stack: TX, V3, V2, V1, A1, A2, A3
  - Track labels with appropriate colors
  - Mute/solo buttons (visual only, functional later)
- [ ] Implement `ui/timeline/playhead.py` — playhead widget:
  - Vertical line spanning all tracks
  - Click on time ruler to seek playhead
  - Drag playhead to scrub
  - Playhead color: accent blue (#4A9EFF)
- [ ] Timeline zoom:
  - `Ctrl+=` / `Ctrl+-` to zoom in/out
  - `Shift+Z` / `Ctrl+Shift+F` to fit all clips in view
  - Zoom slider in toolbar
  - Zoom centers on playhead position

**Dependencies:** Week 3 (UI shell, main window layout).

**Exit criteria:** Timeline appears below panels. Track headers visible. Playhead
can be clicked and dragged. Zoom works.

---

### Week 6: Clip Rendering & Drag-Drop

**Deliverables:**
- [ ] Implement `ui/timeline/clip_item.py` — video/audio clip block:
  - QGraphicsRectItem subclass
  - Video clips: colored rectangle (V1=#3A5A8A, V2=#2E4A7A, V3=#223A6A)
  - Audio clips: colored rectangle (A1=#2E6A4A, A2=#225A3A, A3=#1A4A2E)
  - Clip label (filename, truncated)
  - Selected state: blue selection highlight
  - Rounded corners (4px border radius)
- [ ] Drag from media bin to timeline:
  - Start drag on media bin item
  - Show ghost preview on timeline while dragging
  - Drop onto a specific track (snap to track lane)
  - Creates new Clip in the timeline data model
  - Clips snap to adjacent clip edges (snapping with visual indicator)
- [ ] Clip selection:
  - Click to select (deselect others)
  - Ctrl+Click to toggle selection (multi-select)
  - Click on empty area to deselect all
- [ ] Implement `core/timeline.py` — timeline operations:
  - `add_clip(track_id, clip)` — insert clip at position
  - `remove_clip(track_id, clip_id)` — remove clip
  - `get_clips_at_time(time)` — query clips at playhead position
  - Overlap detection and prevention

**Dependencies:** Week 5 (timeline foundation), Week 2 (media items with proxies).

**Exit criteria:** Can drag a clip from the media bin onto the timeline. Clips
render as colored blocks with filename labels. Click to select works.

---

### Week 7: Core Editing Operations

**Deliverables:**
- [ ] Implement `core/edit_history.py` — undo/redo system:
  - Abstract `EditCommand` base class: `execute()`, `undo()`, `description`
  - `EditHistory` class: undo stack, redo stack, max depth (100)
  - Group commands for batch operations
  - Signal emission on push/undo/redo
- [ ] Concrete commands:
  - `AddClipCommand` — add clip to track
  - `DeleteClipCommand` — remove clip (stores clip for undo)
  - `MoveClipCommand` — move clip to new position/track
  - `SplitClipCommand` — blade tool split
  - `TrimClipCommand` — adjust in/out points
- [ ] Blade tool (B key):
  - Switch to blade mode
  - Click on timeline or press B → split clip at playhead position
  - Creates two clips from one, preserving source in/out points
  - Pushes `SplitClipCommand` to undo stack
- [ ] Delete operations:
  - `Delete` key → ripple delete: remove clip, shift subsequent clips left
  - `Backspace` key → lift delete: remove clip, leave gap
- [ ] Trim handles:
  - Hover near clip edge → cursor changes to trim cursor
  - Drag left edge → adjust source in point
  - Drag right edge → adjust source out point
  - Visual feedback during trim (time readout)
- [ ] Clip move/reorder:
  - Drag clip to new position on same track
  - Snap to adjacent clips and playhead
  - Pushes `MoveClipCommand` to undo stack

**Dependencies:** Week 6 (clip items on timeline).

**Exit criteria:** Blade tool splits clips. Delete works (ripple and gap). Trim
handles adjust clip edges. Undo/redo works for all operations.

---

### Week 8: Copy/Paste, Inspector & Visual Enhancements

**Deliverables:**
- [ ] Cut/Copy/Paste:
  - `Ctrl+C` copies selected clip(s) to clipboard (internal)
  - `Ctrl+X` copies and deletes (ripple)
  - `Ctrl+V` pastes at playhead on the same track as the source clip
  - Multi-clip copy/paste
- [ ] Implement `ui/panels/inspector.py` — inspector router:
  - Detects selection type (video clip, audio clip, text block, nothing)
  - Routes to appropriate sub-inspector
- [ ] Implement `ui/panels/inspector_clip.py` — clip properties:
  - Clip name (read-only)
  - Duration display
  - Source In / Out timecodes
  - File path
  - Resolution and FPS (video only)
- [ ] Implement `ui/timeline/waveform_widget.py` — waveform painter:
  - Draws audio waveform from peak data
  - Waveform color: #5AE8A0 (bright green)
  - Scales with zoom level
- [ ] Thumbnail strip on video clips:
  - Extract multiple thumbnails across clip duration
  - Render as strip inside clip block
  - Update on zoom level change
- [ ] Clip drag between tracks:
  - Drag clip from V1 to V2 (or any valid track of same type)
  - Video clips can only be on V1/V2/V3
  - Audio clips can only be on A1/A2/A3

**Dependencies:** Week 7 (editing operations, undo stack).

**Exit criteria:** Inspector shows clip properties. Waveforms visible on audio
clips. Thumbnails visible on video clips. Copy/paste works.

---

### Week 9: Playback Integration & All Keybindings

**Deliverables:**
- [ ] Timeline ↔ Preview sync:
  - Moving playhead on timeline seeks preview player
  - Preview playback moves playhead on timeline
  - Pressing play in preview advances playhead in real-time
- [ ] Implement `ui/keybindings.py` — centralized keybinding registration:
  - All shortcuts from KEYBINDINGS.md registered here
  - Context-aware (global vs. timeline-only vs. clip-selected)
  - No QShortcut created anywhere else
- [ ] J/K/L shuttle connected to timeline:
  - J/K/L controls preview player speed and direction
  - Playhead moves with shuttle playback
- [ ] Implement `ui/timeline/upper_timeline.py` — overview timeline:
  - Miniature rendering of entire project
  - Viewport rectangle shows what's visible in the lower timeline
  - Click on upper timeline to jump playhead
- [ ] Rubber-band selection:
  - Click and drag on empty timeline area → rubber band rectangle
  - Select all clips within rectangle
  - Shift+click to add to selection

**Dependencies:** Week 8 (inspector, visual enhancements), Week 4 (preview player).

**Exit criteria:** Preview and timeline are fully synchronized. All keybindings
work. Upper timeline shows project overview. Can edit a multi-clip timeline,
preview the result, and save the project.

---

### Phase 2 Exit Criteria ✅

> **Can edit a multi-clip timeline, preview the edit, save the project.**
>
> Full editing workflow: drag clips to timeline, blade tool, trim, delete,
> copy/paste, undo/redo. Preview syncs with timeline. All keybindings active.

---

## Phase 3 — Transitions + Speed + Text + Export (Weeks 10–14)

**Goal:** Produce an actual output file with transitions, speed changes, text
overlays, and proper export settings.

### Week 10: Transitions

**Deliverables:**
- [ ] Transition UI in `inspector_clip.py`:
  - Transition In dropdown: Cut, Fade In, Cross Dissolve, Cut to Black, Cut to White
  - Transition Out dropdown: Cut, Fade Out, Cross Dissolve, Cut to Black, Cut to White
  - Crossfade option (audio clips only)
  - Transition duration input (0.1s – 5.0s, default 1.0s)
- [ ] Implement `core/timeline.py` — transition validation:
  - Cross dissolve requires overlapping clip handles (source media beyond in/out)
  - Warn if insufficient handles
- [ ] Visual indicator on timeline clips:
  - Diagonal gradient at clip edge showing transition type
  - Duration indicator
- [ ] FFmpeg transition rendering (export path, not preview):
  - `xfade` filter for video transitions
  - `acrossfade` for audio transitions
  - Fade in/out via `fade` filter
  - Cut to black/white via color source + xfade
- [ ] `AddTransitionCommand` for undo support

**Dependencies:** Week 8 (inspector panel).

**Exit criteria:** Can set transitions on clips via inspector. Visual indicator
appears on timeline. Transitions are stored in project file.

---

### Week 11: Speed Control

**Deliverables:**
- [ ] Implement `core/speed.py` — speed multiplier logic:
  - Speed steps: [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 4.0]
  - `next_speed(current)` and `prev_speed(current)` step functions
  - `adjusted_duration(original_duration, speed)` → timeline duration
  - Duration recalculation when speed changes
- [ ] Speed UI in `inspector_clip.py`:
  - Speed dropdown (0.25× – 4×)
  - Pitch correction toggle checkbox (on by default)
- [ ] Speed shortcuts:
  - `Ctrl+R` → open speed change dialog (modal)
  - `Ctrl+Up` → increase speed one step
  - `Ctrl+Down` → decrease speed one step
- [ ] Timeline visual updates:
  - Clip width auto-adjusts when speed changes
  - Speed badge on clip block (e.g., "2×" or "0.5×") when not 1×
  - Badge uses contrasting colors for visibility
- [ ] FFmpeg speed rendering (export path):
  - Video: `setpts=PTS/{speed}` filter
  - Audio (with pitch correction): `atempo={speed}` (chained for values outside 0.5–2.0 range)
  - Audio (without pitch correction): `asetrate={rate*speed},aresample={rate}`
- [ ] `ChangeSpeedCommand` for undo support

**Dependencies:** Week 8 (inspector), Week 10 (transition infrastructure).

**Exit criteria:** Can change clip speed via inspector or shortcuts. Timeline
reflects speed visually. Speed badge appears. Duration adjusts correctly.

---

### Week 12: Text/Title Overlays

**Deliverables:**
- [ ] Implement `core/text_overlay.py` — text clip logic:
  - TextClip dataclass validation
  - Default values: Inter 36px, white, center, position (50%, 85%), no rotation
  - Font enumeration (system fonts)
- [ ] TX track on timeline:
  - Always visible above V3
  - Track header labeled "TX" with purple accent
  - Track background slightly differentiated
- [ ] Text block creation:
  - Double-click on TX track → create text block at click position
  - `Ctrl+T` → create text block at playhead position
  - Default duration: 5 seconds
  - Pushes `AddTextCommand` to undo stack
- [ ] Implement `ui/timeline/text_clip_item.py` — text clip rendering:
  - Purple rectangle (#6A3A8A)
  - Text preview inside block (truncated if too long)
  - Drag to move, drag edges to trim duration
- [ ] Implement `ui/panels/inspector_text.py` — text inspector:
  - Text content (QTextEdit, multiline)
  - Font family dropdown (Inter, Arial, Roboto, Times New Roman, monospace)
  - Font size (QSpinBox, 8–120px)
  - Font color (QColorDialog picker)
  - Background color + opacity slider (0–100%)
  - Bold / Italic / Underline toggle buttons
  - Alignment: Left / Center / Right radio buttons
  - Position X: QSlider (0–100%, default 50%)
  - Position Y: QSlider (0–100%, default 85%)
  - Rotation: QSlider (-180° to +180°) + QSpinBox
- [ ] Text export via FFmpeg:
  - `drawtext` filter with all properties (font, size, color, position, etc.)
  - Multiple text blocks → chained drawtext filters
- [ ] Live text preview in MPV:
  - Render text overlay on proxy stream during preview playback
  - Slight delay acceptable (text composited via FFmpeg drawtext subprocess)
- [ ] `AddTextCommand`, `ModifyTextCommand`, `DeleteTextCommand` for undo

**Dependencies:** Week 9 (keybindings), Week 8 (inspector infrastructure).

**Exit criteria:** TX track visible. Can create, edit, and delete text blocks.
Text properties editable in inspector. Text preview renders in player.

---

### Week 13: Export System

**Deliverables:**
- [ ] Implement `media/exporter.py` — export engine:
  - Build FFmpeg filter graph from complete timeline state
  - Handle: clip concatenation, transitions (xfade), speed changes (setpts/atempo),
    text overlays (drawtext), multi-track compositing
  - Use **original files** (not proxies) for export
  - Support output resolutions: original, 1080p, 720p, 480p
  - Output format: MP4 (H.264 + AAC)
  - Quality presets: high (CRF 18), medium (CRF 23), low (CRF 28)
- [ ] Export dialog:
  - Output file path (with file browser)
  - Resolution dropdown
  - Quality preset dropdown
  - Estimated file size (rough calculation)
  - Export / Cancel buttons
- [ ] Export worker (QThread):
  - Run FFmpeg export in background
  - Parse FFmpeg stderr for progress (frame count, time, speed)
  - Emit progress signals to UI
- [ ] Export progress UI:
  - Modal progress dialog
  - Progress bar (0–100%)
  - Time elapsed / estimated remaining
  - Cancel button (kill FFmpeg process)
  - Success notification with "Open File" and "Open Folder" buttons

**Dependencies:** Week 10 (transitions), Week 11 (speed), Week 12 (text).

**Exit criteria:** Can export a complete project to MP4 with transitions, speed
changes, and text overlays. Progress bar shows accurate progress.

---

### Week 14: Polish & Packaging

**Deliverables:**
- [ ] Recent projects list:
  - Store in `~/.tempo/recent_projects.json`
  - Show on startup if no project open
  - File menu → "Recent Projects" submenu
  - Max 10 recent projects, validate existence on display
- [ ] Auto-save:
  - Auto-save every 2 minutes to `{project_dir}/.tempo_autosave.json`
  - Prompt to recover on next launch if autosave exists
  - Clear autosave on clean exit
- [ ] PyInstaller packaging:
  - Create `scripts/build.sh` for Linux
  - Bundle MPV, fonts, icons
  - Single-file executable
  - Test on clean system
- [ ] Create `scripts/dev.sh` — development environment setup:
  - `uv sync --dev`
  - Check for FFmpeg and MPV installation
  - Run linter and type checker
- [ ] Integration testing:
  - End-to-end test: import → edit → export
  - Test all transitions
  - Test speed changes
  - Test text overlays
  - Test project save/load roundtrip

**Dependencies:** All previous weeks.

**Exit criteria:** App runs as a single binary via PyInstaller. Auto-save works.
Recent projects list is functional.

---

### Phase 3 Exit Criteria ✅

> **Can export a finished video with transitions, speed changes, and text
> overlays. App runs as a single binary.**
>
> All features from the MUST HAVE list are implemented and working.

---

## Phase 4 — Polish + Performance (Weeks 15+)

**Goal:** Feels like a real app. Smooth UX, good error handling, optimized
performance on low-end hardware.

### Planned Work (not strictly week-by-week)

- [ ] **Onboarding screen** — first-launch welcome with quick tips
- [ ] **Drag and drop from file manager** — accept drops on media bin and timeline
- [ ] **Missing media re-link** — dialog to re-locate moved/renamed source files
- [ ] **Proxy regeneration** — detect if source file changed (size/mtime) and re-generate
- [ ] **Rust hot paths via PyO3** — profile first, then optimize:
  - Waveform peak calculation
  - Timeline layout math (clip position calculations)
  - Possibly: thumbnail strip generation
- [ ] **Keyboard shortcut cheat sheet** — `?` key opens translucent overlay
- [ ] **Error handling improvements**:
  - FFmpeg missing → helpful install instructions
  - MPV missing → helpful install instructions
  - Disk full → warning before export
  - Corrupt project file → recovery options
- [ ] **Memory management** — unload proxy files for media not currently on timeline
- [ ] **Performance profiling** — identify and fix any remaining bottlenecks
- [ ] **Cross-platform testing** — verify on Windows and macOS

---

## Dependency Graph

```
Week 1 ─── Week 2 ─── Week 3 ─── Week 4
  │           │           │           │
  │           │           └───────────┤
  │           │                       │
  │           └───────────────────────┤
  │                                   │
  └───────────────────────────────────┤
                                      │
                              Week 5 ─┤
                                │     │
                              Week 6  │
                                │     │
                              Week 7  │
                                │     │
                              Week 8 ─┘
                                │
                              Week 9
                                │
                    ┌───────────┤
                    │           │
                 Week 10    (Week 9)
                    │           │
                 Week 11       │
                    │           │
                 Week 12 ──────┘
                    │
                 Week 13
                    │
                 Week 14
                    │
                 Phase 4
```

---

## Risk Mitigation

| Risk | Mitigation |
|------|------------|
| MPV embedding issues with Qt6 | Spike test in Week 3; fallback: QMediaPlayer |
| FFmpeg filter graph complexity for export | Prototype export pipeline early in Week 10 |
| Performance on large projects | Profile in Phase 4; Rust hot paths if needed |
| PyInstaller bundling MPV/FFmpeg | Test packaging early in Week 13 |
| Cross-platform font rendering | Use bundled fonts (Inter, JetBrains Mono) |

---

## Success Metrics

| Metric | Target |
|--------|--------|
| Cold start time | < 3 seconds |
| Import 1GB video (proxy gen) | < 60 seconds |
| Timeline scrubbing latency | < 100ms |
| Export speed (relative to real-time) | > 0.5× on modern laptop |
| Memory usage (10-clip project) | < 500 MB |
| Binary size (PyInstaller) | < 200 MB |
