# MASTER PLANNING PROMPT
## Tempo — Lightweight Video Editor (DaVinci Resolve Cut Page Clone)
### Paste this entire prompt into your AI IDE at project start

---

## WHO YOU ARE

You are a senior software architect and product designer helping me build a
professional-grade, lightweight, CPU-only desktop video editor called **Tempo**.
You will first generate all planning and specification documents before writing
any code. Do NOT write implementation code until I explicitly say "start coding."

---

## WHAT I AM BUILDING

A **lightweight desktop video editor** that clones the functionality and
keybindings of **DaVinci Resolve's Cut Page (page 2)** — stripped down to only
the essentials. Target users are complete beginners who find DaVinci too heavy
and complicated.

**Core philosophy:**
- CPU-only. No GPU required. Runs on any modern laptop.
- Proxy-based editing. Never touch the original file during editing.
- Beginner-friendly. If a beginner can't figure it out in 30 seconds, it's too complex.
- Fast. Feels snappy even on low-end hardware.
- Visually clean. Dark theme, professional look, not cluttered.

---

## TECH STACK (FIXED — DO NOT SUGGEST ALTERNATIVES)

```
Language:        Python 3.11+ (primary)
                 Rust via PyO3 (added later in Phase 4 only for hot paths)
UI Framework:    PySide6 (Qt6)
Video Engine:    FFmpeg via subprocess / ffmpeg-python
Preview Player:  python-mpv (embedded MPV player)
Data Model:      Python dataclasses + JSON (project files)
API Layer:       FastAPI + Uvicorn (only if needed — see note below)
Packaging:       PyInstaller (single binary distribution)
Build Tool:      uv (fast Python package manager)
Linter:          ruff
Type Checker:    mypy
```

**When to use FastAPI (only if these needs arise — do not add it by default):**
- A local HTTP API is needed so external tools or scripts can talk to Tempo
  (e.g. a CLI tool that triggers export, or a future web-based remote control)
- A background worker process needs to communicate with the UI process
  (e.g. a separate FFmpeg worker process sending progress events back)
- A plugin system is added where third-party tools POST to Tempo's local server

**If FastAPI is added**, run it as a local-only server on `127.0.0.1` on a
random available port. Never expose it on `0.0.0.0`. Bundle Uvicorn inside
the PyInstaller binary. Keep it in `tempo/api/` as a self-contained module.

**Do NOT add FastAPI just to move logic out of Python into HTTP calls.**
The core editor is a desktop app — Qt signals handle internal communication.
FastAPI is for cross-process or external-tool integration only.

---

## FEATURES — EXACT SCOPE (NOTHING MORE, NOTHING LESS)

### MUST HAVE (MVP)
- [ ] Import video files (MP4, MOV, MKV, AVI, WebM)
- [ ] Import audio files (MP3, WAV, AAC, FLAC, OGG)
- [ ] Import image files (JPG, PNG, WebP) as static clips
- [ ] Auto-generate proxy files on import (480p for 1080p+, 360p for 4K+)
- [ ] Media Bin (left panel) — shows imported files with thumbnail
- [ ] Dual Timeline (exactly like Resolve Cut Page):
  - Upper timeline: full project overview (zoom out)
  - Lower timeline: detailed editing view (zoom in)
- [ ] Multi-track: 3 video tracks (V1, V2, V3) + 3 audio tracks (A1, A2, A3)
- [ ] Clip operations: Cut, Copy, Paste, Delete, Trim (in/out points)
- [ ] Preview player (center) with transport controls
- [ ] Waveform display on audio clips
- [ ] Thumbnails on video clips

### TRANSITIONS (applied at export via FFmpeg)
- [ ] Cut (default, no transition)
- [ ] Fade In (from black)
- [ ] Fade Out (to black)
- [ ] Cross Dissolve
- [ ] Cut to Black
- [ ] Cut to White
- [ ] Crossfade (audio only)

### SPEED CONTROL (applied to video clips)
- [ ] Speed multiplier per clip: 0.25x, 0.5x, 0.75x, 1x (normal), 1.25x, 1.5x, 2x, 4x
- [ ] Speed is set via a dropdown in the Inspector panel when a video clip is selected
- [ ] Clip block on timeline visually shows speed badge (e.g. "2x" or "0.5x") when not at 1x
- [ ] Clip duration on timeline auto-adjusts when speed changes
  (e.g. a 10s clip at 2x becomes 5s wide on the timeline)
- [ ] Audio pitch correction toggle (on by default — keeps pitch natural at changed speed)
- [ ] Speed is baked into the export via FFmpeg `setpts` + `atempo` filters
- [ ] No keyframe-based ramping — speed is a single constant value per clip (keep it simple)

### TEXT / TITLE OVERLAYS
- [ ] A dedicated Text track (TX) sits above V3 on the timeline — always visible
- [ ] Add a text block by double-clicking on the TX track at any position
- [ ] Text blocks appear as purple clip blocks on the TX track
- [ ] Each text block has:
    - Text content (multiline supported)
    - Font family (dropdown — system fonts: Inter, Arial, Roboto, Times New Roman, monospace)
    - Font size (8px – 120px, numeric input + up/down arrows)
    - Font color (color picker)
    - Background color + opacity (optional backing box, default: transparent)
    - Bold / Italic / Underline toggles
    - Alignment: Left / Center / Right
    - Position on screen: X% and Y% (0–100 sliders, default center: 50%, 85%)
    - Rotation: -180° to +180° (slider + numeric input)
    - Duration: drag to extend/shorten on the timeline like any clip
- [ ] Text is rendered at export via FFmpeg `drawtext` filter
- [ ] In preview: text overlay is rendered live on the preview player using
  FFmpeg drawtext on the proxy stream (can be slightly delayed — acceptable)
- [ ] Text blocks are NOT clips in the media bin — they are created directly on the TX track
- [ ] Selecting a text block in the TX track shows its properties in the Inspector panel

### EXPORT
- [ ] Export to MP4 (H.264, AAC) — uses original files, not proxies
- [ ] Export resolution: original, 1080p, 720p, 480p
- [ ] Progress bar during export

### PROJECT
- [ ] Save project (.tempo JSON file)
- [ ] Load project
- [ ] Recent projects list

### MUST NOT HAVE (explicitly out of scope — refuse to add these)
- Color grading / color wheels
- Effects plugins
- Green screen / chroma key
- Motion tracking
- GPU acceleration (keep it CPU-only)
- Cloud sync
- Collaboration features
- Audio mixing / EQ
- Any feature not listed above

---

## KEYBINDINGS — EXACTLY MATCHING DAVINCI RESOLVE CUT PAGE

```
PLAYBACK
  Space          Play / Pause
  J              Rewind (press multiple times to speed up)
  K              Stop / Pause
  L              Fast Forward (press multiple times to speed up)
  Left Arrow     Step back 1 frame
  Right Arrow    Step forward 1 frame
  Shift+Left     Step back 1 second
  Shift+Right    Step forward 1 second
  Home           Go to start
  End            Go to end

IN / OUT POINTS
  I              Mark In point
  O              Mark Out point
  Alt+I          Clear In point
  Alt+O          Clear Out point

EDITING
  Ctrl+Z         Undo
  Ctrl+Shift+Z   Redo
  Ctrl+C         Copy clip
  Ctrl+X         Cut clip
  Ctrl+V         Paste clip
  Delete         Delete selected clip (ripple delete)
  Backspace      Delete selected clip (leave gap)
  B              Blade / Razor tool (split clip at playhead)
  A              Select / Arrow tool
  T              Trim tool

SPEED CONTROL (when a video clip is selected)
  Ctrl+R         Open speed change dialog for selected clip
  Ctrl+Up        Increase speed one step (e.g. 1x → 1.25x → 1.5x → 2x → 4x)
  Ctrl+Down      Decrease speed one step (e.g. 1x → 0.75x → 0.5x → 0.25x)

TEXT / TITLE
  Ctrl+T         Add new text block at playhead position on TX track
                 (or double-click directly on TX track)

TIMELINE
  Ctrl++         Zoom in timeline
  Ctrl+-         Zoom out timeline
  Ctrl+Shift+F   Fit timeline to window (zoom to fit)
  Shift+Z        Zoom timeline to fit

IMPORT / EXPORT
  Ctrl+I         Import media
  Ctrl+E         Export / Render
  Ctrl+S         Save project
  Ctrl+Shift+S   Save project as
  Ctrl+O         Open project

MEDIA BIN
  F5             Refresh media bin
```

---

## UI LAYOUT — EXACT PANEL STRUCTURE

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  MENU BAR: File | Edit | View | Export | Help           [minimize][max][x]  │
├────────────────┬────────────────────────────────┬───────────────────────────┤
│                │                                │                           │
│   MEDIA BIN    │        PREVIEW PLAYER          │    INSPECTOR PANEL        │
│                │                                │                           │
│  [thumbnail]   │   ┌──────────────────────┐     │  ── VIDEO CLIP ─────────  │
│  filename.mp4  │   │                      │     │  Name: clip.mp4           │
│  00:02:34      │   │     VIDEO PREVIEW    │     │  Duration: 00:01:23       │
│                │   │   (text overlays     │     │  In:  00:00:05            │
│  [thumbnail]   │   │    shown live here)  │     │  Out: 00:01:28            │
│  audio.mp3     │   └──────────────────────┘     │                           │
│  00:01:12      │                                │  Speed: [1x         ▼]    │
│                │   [|◄] [◄◄] [►/||] [►►] [►|]  │  ☑ Pitch correction       │
│  [thumbnail]   │        00:01:23 / 00:05:00     │                           │
│  photo.jpg     │                                │  Transition:              │
│  image         │                                │  [Cross Dissolve    ▼]    │
│                │                                │  Duration: [1.0s]         │
│  [+ Import]    │                                │                           │
│                │                                │  ── TEXT BLOCK ─────────  │
│                │                                │  (shown when TX selected) │
│                │                                │  Text: [______________]   │
│                │                                │  Font: [Inter       ▼]    │
│                │                                │  Size: [36 ▲▼]  [B][I][U] │
│                │                                │  Color: [■] Bg: [■] 0%    │
│                │                                │  Align: [◄][■][►]         │
│                │                                │  X: [50%══════] Y:[85%══] │
│                │                                │  Rotation: [0°════════]   │
├────────────────┴────────────────────────────────┴───────────────────────────┤
│  UPPER TIMELINE (full project overview — zoomed out)                        │
│  ████░░░████░████░░░████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  ▲      │
│                                                                       │      │
├───────────────────────────────────────────────────────────────────────│──────┤
│  LOWER TIMELINE (detailed editing — zoomed in)                        │      │
│                                                                       │      │
│  TX │        [ Title Text Block ]                                     │      │
│  V3 │                                                                 │      │
│  V2 │        [  clip B  ]         [  clip C  ]                       │      │
│  V1 │  [ clip A 2x ]   [  clip B  ]           [   clip D   ]        │      │
│  A1 │  [▓▓▓▓▓▓▓▓▓▓▓]   [▓▓▓▓▓▓▓▓]             [▓▓▓▓▓▓▓▓▓▓▓]        ▼      │
│  A2 │                                                                        │
│  A3 │                                                                        │
│                                                                              │
│  TOOLBAR: [Select A] [Blade B] [Trim T] [Text Ctrl+T]                       │
│           Zoom: [-][========][+]  [Fit]                                      │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## UI DESIGN SYSTEM

**Theme:** Dark professional (like DaVinci Resolve, not like iMovie)

```
Color Palette:
  Background:       #1A1A1F   (near-black, main app bg)
  Panel BG:         #242429   (slightly lighter, panels)
  Surface:          #2E2E35   (cards, clip blocks)
  Border:           #3A3A42   (subtle separators)
  Accent:           #4A9EFF   (primary action blue, playhead, selections)
  Accent Hover:     #6AB4FF
  Text Primary:     #E8E8EE   (main text)
  Text Secondary:   #8A8A95   (labels, timestamps)
  Text Disabled:    #55555E
  Success:          #4CAF82   (export complete)
  Warning:          #E8A43A   (proxy generating)
  Error:            #E85A4A   (missing file)

  Video Track V1:   #3A5A8A   (blue-tinted clip blocks)
  Video Track V2:   #2E4A7A
  Video Track V3:   #223A6A
  Audio Track A1:   #2E6A4A   (green-tinted clip blocks)
  Audio Track A2:   #225A3A
  Audio Track A3:   #1A4A2E
  Image Clip:       #6A3A8A   (purple-tinted)
  Waveform:         #5AE8A0   (bright green waveform on audio clips)
  Playhead:         #4A9EFF   (same as accent)
  Selection:        rgba(74, 158, 255, 0.25)  (translucent blue)

Typography:
  UI Font:          Inter (system fallback: Segoe UI, SF Pro, sans-serif)
  Monospace:        JetBrains Mono (timecodes, file sizes)
  Base size:        13px
  Timeline labels:  11px
  Timecode:         14px monospace

Border Radius:
  Clips on timeline: 4px
  Buttons:           6px
  Panels:            0px (flush edges)
  Dialogs:           8px

Spacing unit: 4px base grid
```

---

## PROJECT FILE FORMAT (.tempo)

```json
{
  "version": "1.0",
  "project_name": "My Project",
  "created_at": "2025-01-01T00:00:00Z",
  "modified_at": "2025-01-01T00:00:00Z",
  "settings": {
    "resolution": [1920, 1080],
    "framerate": 30,
    "sample_rate": 48000
  },
  "media": [
    {
      "id": "uuid-1",
      "original_path": "/home/user/videos/clip.mp4",
      "proxy_path": "/home/user/.tempo/proxies/abc123.mp4",
      "type": "video",
      "duration": 94.5,
      "width": 1920,
      "height": 1080,
      "fps": 30.0,
      "has_audio": true,
      "thumbnail_path": "/home/user/.tempo/thumbs/abc123.jpg"
    }
  ],
  "timeline": {
    "duration": 120.0,
    "tracks": [
      {
        "id": "V1",
        "type": "video",
        "index": 0,
        "clips": [
          {
            "id": "clip-uuid-1",
            "media_id": "uuid-1",
            "timeline_start": 0.0,
            "timeline_end": 10.0,
            "source_in": 5.0,
            "source_out": 15.0,
            "speed": 1.0,
            "pitch_correction": true,
            "transition_in": {
              "type": "cross_dissolve",
              "duration": 1.0
            },
            "transition_out": null
          }
        ]
      }
    ],
    "text_clips": [
      {
        "id": "text-uuid-1",
        "timeline_start": 5.0,
        "timeline_end": 10.0,
        "content": "Hello World",
        "font_family": "Inter",
        "font_size": 36,
        "font_color": "#FFFFFF",
        "background_color": "#000000",
        "background_opacity": 0.0,
        "bold": false,
        "italic": false,
        "underline": false,
        "alignment": "center",
        "position_x": 50.0,
        "position_y": 85.0,
        "rotation": 0.0
      }
    ]
  }
}
```

---

## DIRECTORY STRUCTURE (generate this exactly)

```
tempo/
├── README.md
├── SPEC.md                    ← Full feature specification (generate this)
├── ARCHITECTURE.md            ← Technical architecture (generate this)
├── ROADMAP.md                 ← Phase-by-phase development plan (generate this)
├── KEYBINDINGS.md             ← Full keybinding reference (generate this)
├── pyproject.toml             ← uv project config
├── .python-version            ← 3.11
├── ruff.toml                  ← Linting config
├── mypy.ini                   ← Type checking config
├── .gitignore
│
├── tempo/                   ← Main Python package
│   ├── __init__.py
│   ├── __main__.py            ← Entry point: python -m tempo
│   ├── app.py                 ← QApplication setup
│   │
│   ├── core/                  ← Business logic, NO Qt imports here
│   │   ├── __init__.py
│   │   ├── models.py          ← Clip, Track, TextClip, Project dataclasses
│   │   ├── project.py         ← Save/load project JSON
│   │   ├── timeline.py        ← Timeline operations (cut, paste, trim)
│   │   ├── text_overlay.py    ← TextClip dataclass + validation logic
│   │   ├── speed.py           ← Speed multiplier steps, duration recalculation
│   │   └── edit_history.py    ← Undo/redo stack
│   │
│   ├── media/                 ← Media handling
│   │   ├── __init__.py
│   │   ├── importer.py        ← File import, metadata extraction
│   │   ├── proxy.py           ← FFmpeg proxy generation (background thread)
│   │   ├── thumbnail.py       ← Thumbnail extraction
│   │   ├── waveform.py        ← Waveform peak data extraction
│   │   └── exporter.py        ← Final export via FFmpeg
│   │
│   ├── ui/                    ← All Qt/PySide6 code
│   │   ├── __init__.py
│   │   ├── main_window.py     ← Main QMainWindow
│   │   ├── theme.py           ← QStyleSheet and color constants
│   │   ├── keybindings.py     ← QShortcut registration
│   │   │
│   │   ├── panels/
│   │   │   ├── __init__.py
│   │   │   ├── media_bin.py        ← Left panel: imported media list
│   │   │   ├── preview.py          ← Center: MPV preview player
│   │   │   ├── inspector.py        ← Right panel: clip properties (routes to sub-inspectors)
│   │   │   ├── inspector_clip.py   ← Inspector view for video/audio clips (speed, transition)
│   │   │   └── inspector_text.py   ← Inspector view for text blocks (font, color, position)
│   │   │
│   │   └── timeline/
│   │       ├── __init__.py
│   │       ├── timeline_widget.py    ← Outer container
│   │       ├── upper_timeline.py     ← Overview timeline (zoomed out)
│   │       ├── lower_timeline.py     ← Detail timeline (zoomed in)
│   │       ├── track_header.py       ← TX/V1/V2/A1 labels on left
│   │       ├── clip_item.py          ← Video/audio clip block (shows speed badge)
│   │       ├── text_clip_item.py     ← Text block on TX track (purple, shows text preview)
│   │       ├── playhead.py           ← Playhead line widget
│   │       └── waveform_widget.py    ← Waveform painter for audio clips
│   │
│   └── utils/
│       ├── __init__.py
│       ├── ffmpeg.py          ← FFmpeg subprocess wrapper
│       ├── timecode.py        ← Time formatting helpers (00:01:23.45)
│       ├── cache.py           ← Proxy/thumbnail cache manager
│       └── logger.py          ← App logging setup
│
│   (OPTIONAL — add only if cross-process communication is needed)
├── tempo/api/                 ← FastAPI local server module
│   ├── __init__.py
│   ├── server.py              ← Uvicorn startup, port binding (127.0.0.1 only)
│   ├── routes/
│   │   ├── project.py         ← GET/POST project state
│   │   └── export.py          ← POST trigger export, GET progress
│   └── models.py              ← Pydantic request/response models
│
├── assets/
│   ├── icons/                 ← SVG icons for toolbar buttons
│   └── fonts/                 ← Inter + JetBrains Mono if bundled
│
├── tests/
│   ├── test_models.py
│   ├── test_timeline.py
│   ├── test_project.py
│   └── test_ffmpeg.py
│
└── scripts/
    ├── build.sh               ← PyInstaller build script
    └── dev.sh                 ← Dev environment setup
```

---

## DEVELOPMENT PHASES

### Phase 1 — Core Foundation (Weeks 1–4)
**Goal: A working skeleton you can actually run**
- [ ] Project setup (uv, ruff, mypy, gitignore)
- [ ] Core data models (Clip, Track, Project)
- [ ] Project save/load (JSON)
- [ ] Undo/redo stack (Command pattern)
- [ ] FFmpeg wrapper (probe, proxy gen, thumbnail)
- [ ] Media import pipeline (background thread)
- [ ] Main window shell (panels, splitters, dark theme)
- [ ] Media bin panel (list with thumbnails)
- [ ] Basic preview player (MPV embedded)

**Exit criteria:** Can import a video, see it in the bin, click play in preview.

### Phase 2 — Timeline + Editing (Weeks 5–9)
**Goal: Actually edit something**
- [ ] Lower timeline canvas (QGraphicsScene)
- [ ] Upper timeline (overview)
- [ ] Clip drag from media bin to timeline
- [ ] Clip drag/reorder on timeline
- [ ] Trim handles (drag clip edges to trim)
- [ ] Blade tool (split clip at playhead, B key)
- [ ] Playhead (click to seek, drag)
- [ ] Timeline zoom (Ctrl+/-)
- [ ] Waveform rendering on audio clips
- [ ] Thumbnail rendering on video clips
- [ ] Inspector panel (clip properties, in/out)
- [ ] All keybindings wired up
- [ ] Transport controls (J/K/L)
- [ ] Selection (click, Ctrl+click, rubber-band)
- [ ] Cut/Copy/Paste/Delete

**Exit criteria:** Can edit a multi-clip timeline, preview the edit, save the project.

### Phase 3 — Transitions + Speed + Text + Export (Weeks 10–14)
**Goal: Produce an actual output file with all features working**
- [ ] Transition UI in inspector panel
- [ ] Transition rendering at export (FFmpeg xfade filter)
- [ ] All 7 transitions working
- [ ] Speed control dropdown in inspector (video clips)
- [ ] Timeline clip width auto-adjusts on speed change
- [ ] Speed badge displayed on clip block when not 1x
- [ ] Speed baked into export via FFmpeg `setpts` + `atempo` filters
- [ ] Pitch correction toggle (FFmpeg `asetrate` + `aresample`)
- [ ] TX text track on timeline (always visible, above V3)
- [ ] Add text block via double-click on TX track or Ctrl+T
- [ ] Text inspector panel (font, size, color, bg, alignment, position, rotation)
- [ ] Text rendered at export via FFmpeg `drawtext` filter
- [ ] Text preview rendered live in preview player (proxy stream)
- [ ] Export dialog (resolution, format, output path)
- [ ] Export progress bar
- [ ] Recent projects list
- [ ] App packaging (PyInstaller)

**Exit criteria:** Can export a finished video with transitions, speed changes,
and text overlays. App runs as a single binary.

### Phase 4 — Polish + Performance (Weeks 13+)
**Goal: Feels like a real app**
- [ ] Onboarding screen for new users
- [ ] Drag and drop files from file manager
- [ ] Missing media re-link
- [ ] Proxy regeneration if file changed
- [ ] Profile slow paths → extract to Rust via PyO3 (waveform, layout math)
- [ ] Keyboard shortcut cheat sheet overlay (? key)
- [ ] Error handling and user-friendly error messages
- [ ] Memory: unload proxies for media not in timeline

---

## CRITICAL CONSTRAINTS FOR THE AI IDE

1. **Never import Qt in core/.** All business logic must be Qt-free and independently testable.

2. **All FFmpeg calls go through `utils/ffmpeg.py`.** Never call subprocess directly elsewhere.

3. **Proxy generation MUST run in a background thread (QThread).** Never block the UI thread.

4. **Timeline state is a pure Python list of Clip dataclasses.** The UI renders from this state; it does not mutate it directly. All mutations go through `core/timeline.py` functions.

5. **Every user action that mutates the timeline must push to the undo stack.** Use the Command pattern in `core/edit_history.py`.

6. **Keybindings are registered in one place only: `ui/keybindings.py`.** Never hardcode QShortcut elsewhere.

7. **Colors and fonts come from `ui/theme.py` only.** Never hardcode hex values in widget files.

8. **Do not use Qt Designer / .ui files.** All UI is code-only for maintainability.

9. **Type hints everywhere.** mypy must pass with strict mode.

10. **No feature creep.** If a feature is not in the MUST HAVE list above, refuse to add it until Phase 4 is complete.

---

## YOUR FIRST TASK

Generate the following documents in order. Do not write any Python code yet.

1. **`SPEC.md`** — Full product specification. Cover every feature in detail.
   Include exact UI behavior for every interaction (what happens when user
   drags a clip, what happens when proxy is still generating, what happens
   when a file is missing, etc.)

2. **`ARCHITECTURE.md`** — Technical architecture document.
   Cover: data flow diagrams, threading model, FFmpeg integration strategy,
   proxy cache design, undo/redo design, timeline rendering approach.

3. **`ROADMAP.md`** — Week-by-week development roadmap broken into the
   4 phases above with specific deliverables per week.

4. **`KEYBINDINGS.md`** — Full keybinding reference table (markdown table)
   matching DaVinci Resolve Cut Page exactly as listed above.

5. **`pyproject.toml`** — uv project config with all dependencies pinned:
   PySide6, ffmpeg-python, python-mpv, fastapi, uvicorn, ruff, mypy, pytest.
   Mark fastapi and uvicorn as optional dependencies under `[project.optional-dependencies]`
   so they are not installed by default — only when the API module is needed.

6. **`.gitignore`** — Appropriate for Python + Rust project.

7. **`README.md`** — Project overview, install instructions (uv), run
   instructions, contributing guide.

After I confirm all documents look correct, then we start with
**Phase 1, Week 1: project setup and core data models.**

---

## EXTRA CONTEXT ABOUT ME

- I am a Python developer with experience in AI/ML, PyO3, and Linux systems
- I will be adding Rust (PyO3) extensions in Phase 4 for hot paths only
- Target platforms: Linux (primary), Windows, macOS
- I want the code to be production quality, not tutorial quality
- Use modern Python: dataclasses, type hints, pathlib, match statements
- Preferred async pattern: QThread for background work, not asyncio

---

*Start with document generation. Say "Ready" when you have read and
understood the entire prompt, then begin with SPEC.md.*
