# Visual Design — Tempo

**Version:** 3.1 (2026-10-05)
**Reference:** DaVinci Resolve 20, from the screenshots in `EX/DVR_UI/` taken on the project
owner's own system. Colours in §2 were sampled from those images.
**Companion documents:** `UI_SPEC.md` (widgets and behaviour), `KEYBINDS.md` (shortcuts).

This document is the single source of truth for how Tempo looks.

### Reference screenshots

| Screen | File in `EX/DVR_UI/` |
|---|---|
| Edit page, single viewer, clips on the timeline | `Edit page with files in just single screen mode.png` |
| Edit page, Media Pool + Effects + Inspector open | `Edit page with files in inspect , media and effect window.png` |
| Edit page, dual viewers, empty | `Edit Page empty with dual screen option.png` |
| Export (Deliver) page with a job | `export page with files.png` |
| Export presets and settings | `Export page youtube export options.png`, `Export Page tiktok or instgram export option.png`, `export page custom export option 1.png` |
| Project Manager | `Project manger Screen.png` |
| Loading | `Loading Screen.png` |

The older `dvr_*.jpg` / `.png` files in that folder are from Resolve 16/17 and are no longer
the reference.

---

## 1. Goal

Tempo is the editor people use *before* DaVinci Resolve, and the editor people keep for quick
social-media videos.

- **Visual memory must transfer.** Panels, buttons and colours sit where Resolve puts them.
  A Tempo user who opens Resolve's Edit page should feel they have seen it before.
- **It must be calmer than Resolve.** Fewer buttons, fewer panels, plain words.

### 1.1 Principles

1. **Same place, fewer things.** Keep Resolve's arrangement. Remove controls; do not move them.
2. **Resolve's own colours.** The same dark greys, the same red-orange accent, the same blue
   video clips and green audio clips.
3. **One accent colour.** It marks the playhead, the selection, the active tool and the
   active page — exactly what Resolve uses it for.
4. **Every icon has a tooltip** with the action's name and key.
5. **No decoration.** Flat fills and 1 px lines. This also keeps redraws cheap.
6. **Quiet until needed.** Panels a beginner rarely uses start closed.

### 1.2 What Tempo keeps from Resolve 20, and what it drops

| Area | Resolve 20 | Tempo |
|---|---|---|
| Menu bar | File, Edit, Trim, Timeline, Clip, Mark, View, Playback, Fusion, Color, Fairlight, Workspace, Help | One menu button (`⋯`) at the right of the top bar. Saves 36 px of height on a laptop |
| Top bar, left | Media Pool, Effects, Index, Sound Library, Keyframes | **Media Pool, Effects** |
| Top bar, centre | Project name, "Edited" | **Same** |
| Top bar, right | Quick Export, Mixer, Metadata, Inspector | **Quick Export, Inspector** |
| Left column | Media Pool on top; Effects below it, running down beside the timeline | **Same** |
| Viewer | Dual by default; single-viewer mode available | **Single by default.** Dual offered on wide windows |
| Viewer header | Zoom %, source duration, timeline name, playhead timecode, overlay menus | Zoom %, timeline name, timecode, one `⋯` menu |
| Transport | First frame, reverse, stop, play, last frame, loop; In/Out buttons at the right | **Same** |
| Timeline toolbar | About 22 controls | 10 controls, same order |
| Timecode box | Large, top-left of the timeline | **Same** |
| Track header | Two rows: destination box and name; lock, auto-select, enable (or S, M and level) | One row: destination box, lock, enable (video) or mute (audio) |
| Track edge colour | Blue strip for video, green for audio | **Same** |
| Clips | Blue video with filmstrip and a name bar; green audio with waveform and a name bar | **Same** colours and name bar; **no thumbnails on video clips** |
| Playhead | Red-orange line with a head in the ruler | **Same** |
| Inspector | Six tabs; each section has an on/off dot, keyframe diamond, reset | Sections only (no tabs); each section has a reset |
| Page bar | Bottom; logo left, seven page icons centre, Home and Settings right | **Same position and shape**, two pages: Edit, Export |
| Deliver page | Settings left, viewer and timeline centre, queue right | **Same** |
| Project Manager | Thumbnail grid; Export/Import bottom-left; New Project/Open bottom-right | **Same**, without the Local/Network/Cloud tabs |
| Buttons | Outlined pills | **Same** |
| Theme | Dark only | **Dark only** |

Two Resolve 20 features shape decisions here: the **automatic vertical layout** for vertical
timelines (§5.10), and the **Keyframes panel** being its own panel, not part of the tracks
(reserved for the keyframes plugin point).

---

## 2. Design tokens

Defined once in `assets/style/tempo.css` with `@define-color`. Custom-drawn widgets (timeline,
scrub bar, meter) look the names up from their style context. No colour is written in Rust.

### 2.1 Colour

Hex values marked ● were sampled from the Resolve 20 screenshots.

```css
/* Surfaces */
@define-color tempo_bg_bar       #17181a;  /* ● top bar, page bar */
@define-color tempo_bg_deep      #1b1b1f;  /* ● timecode box, Inspector body, viewer surround */
@define-color tempo_bg_panel     #212126;  /* ● viewer header, transport, toolbar, track headers, dialogs */
@define-color tempo_bg_surface   #28282e;  /* ● timeline body, Media Pool, settings panels */
@define-color tempo_bg_card      #2f3136;  /* ● render-job cards, project cards without a thumbnail */
@define-color tempo_bg_inset     #131316;  /* ● Effects list items */
@define-color tempo_bg_field     #1f1f1f;  /* ● text entries, number fields, drop-downs */
@define-color tempo_bg_active    #000000;  /* ● active page button */
@define-color tempo_bg_hover     #34353b;  /*   hover on rows, cards, flat buttons */

/* Lines */
@define-color tempo_line         #0e0e10;  /*   between panels */
@define-color tempo_line_soft    #3a3a3a;  /* ● video/audio divider, track rows */
@define-color tempo_line_button  #43474d;  /* ● pill button outline */

/* Text */
@define-color tempo_text_bright  #ffffff;  /* ● active tab or button label, project name */
@define-color tempo_text         #d0d0d0;  /* ● values, clip names, headings */
@define-color tempo_text_dim     #929292;  /* ● labels, inactive buttons, "Edited" */
@define-color tempo_text_off     #5c5c62;  /*   disabled */

/* Accent */
@define-color tempo_accent       #e64b3d;  /* ● playhead, selection, active tool, active page line */
@define-color tempo_accent_soft  alpha(#e64b3d, 0.18);

/* Status — always with an icon or a word */
@define-color tempo_ok           #2c9418;  /* ● Resolve's green (volume bar) */
@define-color tempo_warn         #e0a030;
@define-color tempo_error        #e64b3d;

/* Timeline */
@define-color tempo_clip_video       #4376a1;  /* ● video clip */
@define-color tempo_clip_audio       #448f64;  /* ● audio clip, name bar */
@define-color tempo_clip_audio_wave  #397955;  /* ● audio clip, behind the waveform */
@define-color tempo_clip_title       #8a6fb0;  /*   title clip */
@define-color tempo_track_edge_video #345b7b;  /* ● strip at the left edge of video tracks */
@define-color tempo_track_edge_audio #356d4d;  /* ● strip at the left edge of audio tracks */
@define-color tempo_clip_text        #ffffff;
@define-color tempo_waveform         alpha(#e8f5ee, 0.90);
@define-color tempo_clip_off         alpha(#28282e, 0.60);  /* wash over a disabled clip */
@define-color tempo_in_out           alpha(#ffffff, 0.08);
```

Contrast: `tempo_text` on `tempo_bg_panel` is about 10:1; `tempo_text_dim` on
`tempo_bg_panel` is about 5.3:1; white on the video and audio clip fills is about 4.6:1 and
4:1. Clip names are therefore drawn in semibold to stay readable on the green.

**Marker colours** (Resolve's set): Blue `#4f8fd6` (default), Cyan `#3fb8c8`,
Green `#4caf7a`, Yellow `#d8b93a`, Red `#d6504a`, Pink `#d870a8`, Purple `#9a72d0`.

### 2.2 Type

The system UI font. Sizes are relative so the system text size and Tempo's font-scale setting
both work.

| Class | Size | Weight | Used for |
|---|---|---|---|
| `.tempo-title` | 2.2em | 700 | Loading screen name |
| `.tempo-heading` | 1.0em | 600 | Panel titles, project name, dialog titles |
| `.tempo-body` | 1.0em | 400 | Everything else |
| `.tempo-small` | 0.85em | 400 | Ruler labels, badges, hints |
| `.tempo-clip` | 0.9em | 600 | Clip names |
| `.tempo-timecode` | 0.95em | 600, tabular figures | Viewer timecodes |
| `.tempo-timecode-big` | 1.7em | 500, tabular figures | Timeline timecode box |

Resolve sets timecodes in its normal UI font with fixed-width digits, not in a typewriter
font. Tempo does the same (`font-feature-settings: "tnum"`).

No all-caps text except the one status word on the loading screen. No fixed pixel font sizes.

### 2.3 Size and spacing

| Item | Value |
|---|---|
| Spacing steps | 4, 8, 12, 16 px |
| Radius: clips, fields | 3 px |
| Radius: cards, thumbnails | 6 px |
| Radius: dialogs | 10 px |
| Radius: buttons | full (pill) |
| Top bar | 44 px |
| Viewer header | 30 px |
| Transport row | 40 px |
| Timeline toolbar | 44 px |
| Ruler / timecode box | 48 px |
| Page bar | 40 px |
| Track header width | 150 px |
| Video track | 60 px default (32–120) |
| Audio track | 54 px default (32–120) |
| Track edge strip | 3 px |
| Left column, Inspector | 300 px default (240–480) |
| Icons | 16 px symbolic; page-bar icons 20 px |
| Button hit area | at least 32 × 32 px |

**Window:** minimum 1024 × 640; default 1366 × 768 or the saved size. The whole Edit page
must be usable at 1366 × 768.

### 2.4 Motion

Panels open and close in 150 ms. Page changes are instant. Nothing animates on the timeline
or viewer. With the desktop's reduce-animation setting on, all transitions are instant.

---

## 3. Screen 1 — Loading

Resolve shows a splash with the product name at the left, the version under it, and a status
line ("STARTING UP") lower left. Tempo uses the same arrangement, without the photograph,
inside the main window.

```
┌──────────────────────────────────────────────────────────────┐
│                                                              │
│                                                              │
│      ▶║  Tempo                                               │
│          0.1                                                 │
│                                                              │
│                                                              │
│      STARTING AUDIO                                          │
│      ━━━━━━━━━━━━━━━━━━━─────────────                        │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

- Background `tempo_bg_deep`. Content sits in the left third, vertically centred.
- Icon and name (`.tempo-title`, `tempo_text_bright`); version below (`tempo_text_dim`).
- Status word in small capitals (`.tempo-small`, `tempo_text_dim`), with a 2 px progress line
  280 px wide in `tempo_accent` below it.
- The status names what is really happening. No fake steps, no minimum display time.
  If start-up takes under 300 ms the screen is skipped.

| Step | Status text | Work |
|---|---|---|
| 1 | CHECKING GRAPHICS | Create the GPU device; test hardware video decode |
| 2 | STARTING AUDIO | Connect to PipeWire |
| 3 | OPENING PROJECTS | Read the project list and thumbnails |
| 4 | LOADING PLUGINS | Read plugin manifests only |

---

## 4. Screen 2 — Project Manager

Reference: `Project manger Screen.png`.

```
┌──────────────────────────────────────────────────────────────────────────┐
│                               ▶║ Tempo                                    │
├──────────────────────────────────────────────────────────────────────────┤
│  Projects                                 ⊕   ──●──   ⇅   ▦  ☰   🔍     │
├──────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│   ┏━━━━━━━━━━━━┓   ┌────────────┐   ┌────────────┐                       │
│   ┃✓           ┃   │            │   │            │                       │
│   ┃  thumbnail ┃   │  thumbnail │   │  thumbnail │                       │
│   ┃            ┃   │            │   │            │                       │
│   ┗━━━━━━━━━━━━┛   └────────────┘   └────────────┘                       │
│     Iceland Vlog     Product Reel     Untitled Project 1                 │
│                                                                          │
│                                                                          │
│  ( Export )  ( Import )                       ( New Project )  ( Open )  │
└──────────────────────────────────────────────────────────────────────────┘
```

- Background `tempo_bg_panel`. Title bar `tempo_bg_bar` with the Tempo name centred.
- **Header row:** "Projects" (`.tempo-heading`) left. Right, in Resolve's order: new folder,
  thumbnail-size slider, sort, grid view, list view, search. Icons in `tempo_text_dim`; the
  active view icon in `tempo_text_bright`.
- **Card:** a 224 × 126 px thumbnail, radius 6, with the name centred below in `.tempo-heading`
  `tempo_text_dim`.
- **Selected card:** 2 px `tempo_accent` border, a small accent triangle with a tick in the
  top-left corner, and the name in `tempo_accent` — exactly as Resolve shows it.
- **Hover:** 1 px `tempo_line_button` border.
- **No thumbnail yet:** `tempo_bg_card` with the Tempo icon in `tempo_text_off`.
- **Missing file:** thumbnail dimmed with a warning icon; tooltip "File not found".
- **Buttons** are outlined pills. Bottom-left: `Export`, `Import`. Bottom-right:
  `New Project`, `Open`. When a project is already open, `Open` reads `Close`, as in Resolve.
- Double-click opens. Right-click: Open, Rename, Duplicate, Show in Files, Move to Trash.
- Empty state: one centred line, "No projects yet", above the `New Project` button.

---

## 5. Screen 3 — Edit page

Reference: `Edit page with files in just single screen mode.png` and
`Edit page with files in inspect , media and effect window.png`.

```
┌──────────────────────────────────────────────────────────────────────────────────────┐
│ ▣ Media Pool │ ✦ Effects            Iceland Vlog │ Edited      ⇪ Quick Export │ ✕ Inspector │ ⋯ │
├───────────────┬────────────────────────────────────────────────┬─────────────────────┤
│ Master        │ 31% ▾          Timeline ▾          01:00:23:15 │ Glacier_Walk.mp4    │
│ ┌───┐ ┌───┐   │ ┌────────────────────────────────────────────┐ │                     │
│ │   │ │   │   │ │                                            │ │ Transform         ↺ │
│ └───┘ └───┘   │ │                                            │ │   Zoom  X 1.000  Y 1.000
│ Vik   Glacier │ │                  viewer                    │ │   Position X 0  Y 0 │
│ ┌───┐ ┌───┐   │ │                                            │ │   Rotation    0.000 │
│ │   │ │   │   │ └────────────────────────────────────────────┘ │ Composite         ↺ │
│ └───┘ └───┘   │ ├──────────────────────▼─────────────────────┤ │   Opacity    100.00 │
│ Sunset Music  │ ▭▾        ⏮  ◀  ■  ▶  ⏭  ⟲              ⇥  ⇤ │ Audio             ↺ │
├───────────────┼────────────────────────────────────────────────┴─────────────────────┤
│ Effects   🔍  │      ↖  ⇔  ▬   ⤓  ⬓  ⇄   ∩  🔗   ⚑▾       ⊟  − ──●── +        🔊 ━━━● │
│ ▾ Toolbox     ├────────────┬─────────────────────────────────────────────────────────┤
│   Transitions │ 01:00:23:15│ 01:00:00:00     01:00:30:00  ▼   01:01:00:00            │
│   Titles      ├────────────┼─────────────────────────────────────────────────────────┤
│   Filters     │▌ V2  🔒 ▣  │            ▛▀▀▀▀▀▀▀▀▜                                   │
│               │▌           │            ▙ Drone  ▟                                   │
│ Dissolve    ⌃ │▌[V1] 🔒 ▣  │ ▛▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▜▛▀▀▀▀▀▀▀▀▀▀▀▀▀▜▛▀▀▀▀▀▀▀▜              │
│ ┌───────────┐ │▌           │ ▙ 🔗 Vik_BlackSand▟▙ 🔗 Glacier   ▟▙ Sunset ▟              │
│ │▣ Cross Dis│ ├────────────┼─────────────────────────────────────────────────────────┤
│ └───────────┘ │▌[A1] 🔒 M  │ ▛∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿▜▛∿∿∿∿∿∿∿∿∿∿∿∿∿▜▛∿∿∿∿∿∿∿▜              │
│ ┌───────────┐ │▌ A2  🔒 M  │ ▛∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿ Music ∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿▜             │
│ │▣ Dip to Co│ │            │                                                         │
├───────────────┴────────────┴─────────────────────────────────────────────────────────┤
│ ▶║ Tempo    ◔ Proxy 42 %                   ✎        🚀                         ⌂   ⚙ │
└──────────────────────────────────────────────────────────────────────────────────────┘
```

### 5.1 Top bar

44 px, `tempo_bg_bar`, includes the window controls.

- **Left:** `Media Pool`, `Effects` — icon and label, separated by 1 px `tempo_line_soft`
  dividers, as in Resolve.
- **Centre:** project name in `.tempo-heading` `tempo_text_bright`; a thin divider; `Edited`
  in `tempo_text_dim` when there are unsaved changes.
- **Right:** `Quick Export`, `Inspector`, then the menu button `⋯`.
- A panel button whose panel is **open** has `tempo_text_bright` icon and label. **Closed** is
  `tempo_text_dim`. There is no background fill in either state — this is how Resolve shows it.

### 5.2 Left column — Media Pool and Effects

Both can be open at once, stacked as in Resolve: **Media Pool above, Effects below**. When
Effects is open, the left column runs the full height of the page, beside the timeline.
A new project opens with the Media Pool only.

**Media Pool** — `tempo_bg_surface`.
- A header row with the bin name ("Master") in `.tempo-heading`, and view and search icons.
- Thumbnails 88 × 50 px, radius 3, name below in `.tempo-small`, cut with `…`.
- A clip used in the timeline has a thin `tempo_accent` line along the bottom of its thumbnail.
- Selected: 2 px `tempo_accent` border.
- Audio files show a waveform thumbnail.
- Empty: a dashed 1 px `tempo_line_button` outline and the line
  "Drop video, audio or images here, or press Ctrl+I".
- The bin list at the left edge appears only after the user creates a second bin.

**Effects** — `tempo_bg_surface`.
- Left: a short category list — Toolbox ▸ Transitions, Titles, Filters. The selected
  category row is `tempo_bg_bar` with `tempo_text_bright`.
- Right: the items of that category in collapsible groups (for example "Dissolve"). Each item
  is a full-width row on `tempo_bg_inset`, 1 px `tempo_line_button` outline, radius 3: a 64 px
  icon cell, a divider, then the name.
- The default transition has a small `tempo_accent` bar at the left of its icon cell.

### 5.3 Viewer

- **Header** (30 px, `tempo_bg_panel`): zoom menu (`31% ▾`) left; timeline name with a
  drop-down arrow centred, `tempo_text_bright`; playhead timecode right in `.tempo-timecode`;
  then the `⋯` menu (playback quality, safe area, dual viewer).
- **Picture** on `tempo_bg_deep`, letterboxed, no border or rounding.
- **Scrub bar:** a 2 px `tempo_line_soft` line with small end stops. The position marker is
  an `tempo_accent` pin above the line, as in Resolve. In/Out range drawn as a brighter
  stretch of the line.
- **Transport row** (40 px, `tempo_bg_panel`), icons in `tempo_text`:
  - left: viewer mode menu;
  - centre: first frame, play reverse, stop, play, last frame, loop;
  - right: Mark In and Mark Out.
- **Badges** over the picture only when true: `PROXY` (top right, `tempo_warn` on a
  70 %-black pill) and a dropped-frame count while playing.
- **Source mode:** the header shows the clip's name in place of the timeline name, and the
  scrub bar covers the clip.
- **Dual viewer** (window 1600 px or wider): source left, timeline right, each with its own
  header, scrub bar and transport, split by a 1 px `tempo_line`. Matches
  `Edit Page empty with dual screen option.png`.

### 5.4 Inspector

`tempo_bg_deep`, 300 px. Closed by default below 1400 px window width.

- **Title row:** clip name in `.tempo-heading` `tempo_text_bright` on `tempo_bg_panel`.
- **Section header:** a 36 px row on `tempo_bg_surface` with the section name in `tempo_text`
  and a reset icon (↺) at the right in `tempo_text_dim`.
- **Rows:** label right-aligned in `tempo_text_dim` ending at 42 % of the width; value fields
  to its right on `tempo_bg_field`, radius 3, text `tempo_text`. Paired values show small
  `X` and `Y` letters before each field. A per-row reset icon sits at the far right.
- Number fields change on horizontal drag and accept typing. Sliders appear only for bounded
  values (Rotation, Opacity, Volume, Pan), drawn as a thin line with a round handle.
- Nothing selected: one centred line, "Select a clip to see its settings".
- Plugin sections look identical; the host draws them.

| Selection | Sections |
|---|---|
| Video clip | Transform (Zoom, Position, Rotation), Composite (Opacity), Audio if linked |
| Audio clip | Audio (Volume, Pan) |
| Title | Title (Text, Font, Size, Colour, Background, Position) |
| Transition | Transition (Type, Duration, Alignment) |
| Marker | Marker (Name, Colour, Note) |

Resolve's tab row (Video, Audio, Effects, Transition, Image, File) and the per-section on/off
dots and keyframe diamonds are left out. Space for a diamond is kept at the right of each row
so the keyframes plugin point can add it later without moving anything.

### 5.5 Timeline toolbar

44 px, `tempo_bg_panel`. Icons 18 px in `tempo_text_dim`; the **active edit mode and any
switched-on toggle are drawn in `tempo_accent` or `tempo_text_bright`**, as Resolve does
(its Selection arrow is red when active). Groups are separated by 1 px `tempo_line_soft`
dividers. Order follows Resolve, left to right:

| Group | Controls | Key |
|---|---|---|
| Edit mode | Selection, Trim, Blade | `A`, `T`, `B` |
| Edit actions | Insert, Overwrite, Replace | `F9`, `F10`, `F11` |
| Options | Snapping, Linked Selection | `N`, `Ctrl+Shift+L` |
| Marker | Marker, with a colour drop-down | `M` |
| Zoom | Zoom to fit, then − slider + | `Shift+Z`, `Ctrl+-`, `Ctrl+=` |

Far right: speaker icon and the monitor volume slider, drawn as a `tempo_ok` green line with
a round handle, as in Resolve.

Left out from Resolve's toolbar: timeline view options, keyframe and voice-over buttons,
dynamic trim, position lock, flags, the two extra zoom presets, DIM.

### 5.6 Timeline

- **Timecode box:** top-left, 150 × 48 px, `tempo_bg_deep`, `.tempo-timecode-big`
  `tempo_text_bright`. Click to type a time.
- **Ruler:** 48 px, `tempo_bg_surface`. Small ticks along the top edge in `tempo_line_soft`;
  timecode labels in `.tempo-small` `tempo_text_dim`, placed to the right of each major tick.
- **Track header:** `tempo_bg_panel`, one row:
  - a 3 px edge strip (`tempo_track_edge_video` or `tempo_track_edge_audio`);
  - the track name (`V1`) in a 30 × 22 px box. The box has a 1 px `tempo_accent` outline on
    the tracks that will receive the next edit (Resolve's destination control); other tracks
    show the name with no outline;
  - a lock icon;
  - an enable icon for video tracks, or an `M` button for audio tracks.
- **Order:** video tracks stack upward and audio tracks downward from a 2 px `tempo_line_soft`
  divider, as in Resolve.
- **Track body:** `tempo_bg_surface`; 1 px `tempo_line` between tracks. A locked track is
  overlaid with a diagonal hatch at 6 % white.
- **Scrollbar:** a thin rounded bar in `tempo_line_button` at the bottom.
- **Audio meter:** a 28 px strip at the right edge with two bars: `tempo_ok` to −12 dB,
  `tempo_warn` to −6 dB, `tempo_error` above, with a peak-hold tick.

### 5.7 Clips

Resolve draws a clip in two bands: a picture or waveform band on top, and a name bar below.
Tempo does the same.

| | Video clip | Audio clip | Title clip |
|---|---|---|---|
| Top band | Plain `tempo_clip_video`, slightly darker than the name bar (88 % brightness) | Waveform in `tempo_waveform` on `tempo_clip_audio_wave` | — |
| Name bar | `tempo_clip_video`, 22 px | `tempo_clip_audio`, 22 px | Whole clip `tempo_clip_title` |
| Name | Link icon, then the name, `.tempo-clip` white, cut with `…` | Same | `T` icon, then the name |
| Outline | 1 px, 30 % black | Same | Same |
| Radius | 3 px | 3 px | 3 px |

- **No thumbnails on timeline clips.** Resolve draws a filmstrip; Tempo does not, because
  decoding frames for every clip is too much work for a weak CPU. The clip is identified by
  its name. (Thumbnails still appear in the Media Pool, one per file, cached on disk.)
  When a track is under 44 px high, clips show the name bar only.
- **Selected:** 2 px `tempo_accent` outline.
- **Disabled (`D`):** `tempo_clip_off` wash; name in italics.
- **Media offline:** `tempo_bg_card` with a diagonal hatch, a warning icon, "Media offline".
- **Trim:** near an edge the pointer changes and the edge shows a 3 px white bar.
- **Transition:** a rounded box across the cut on the video track, 60 % white outline on a
  40 %-black fill, with the transition's name when there is room.
- **Fade:** a small white handle in each top corner on hover; dragging it draws the fade as a
  darkened triangle over the clip.
- **Link icon** is dimmed when linked selection is off.

### 5.8 Playhead, markers, ranges

- **Playhead:** 1 px `tempo_accent` line through all tracks with a 13 px pointed head in the
  ruler.
- **Marker:** a small flag-shaped tab hanging from the top of the ruler in its marker colour.
  A named marker shows its name in a tooltip.
- **In–Out range:** `tempo_in_out` band across the ruler and tracks.
- **Snap:** a 1 px white line at the snap point while dragging.
- **Drop preview:** a ghost clip at 50 % opacity where a dragged clip will land.

### 5.9 Page bar

40 px, `tempo_bg_bar`, at the bottom, as in Resolve.

- **Left:** the Tempo icon and name in `.tempo-heading` `tempo_text` — where Resolve shows
  "DaVinci Resolve 20". The activity line follows it: empty when idle; otherwise one line
  with a small progress ring (proxy, export, "Saved", or an error). Clicking it lists running
  jobs with cancel buttons.
- **Centre:** two page buttons, each 150 px wide, icon only (pencil-and-timeline for Edit,
  rocket for Export — Resolve's own symbols). The **active** page has a `tempo_bg_active`
  (black) background, a `tempo_text_bright` icon, and a 2 px `tempo_accent` line along its
  **bottom** edge. The other icon is `tempo_text_dim`. Tooltips give the page name and key.
- **Right:** Home (Project Manager) and Settings icons.

### 5.10 Vertical projects

When the project is taller than it is wide, the page rearranges as Resolve 20 does: the
viewer becomes a full-height column on the right (about 30 % of the window), and the Media
Pool, Inspector and timeline share the space to its left. Toolbar, tracks and keys do not
change. The Export page does the same.

```
┌──────────────────────────────────────────────────────────┬───────────────────┐
│ ▣ Media Pool │ ✦ Effects      Reel │ Edited              │ ⇪ │ ✕ Inspector │⋯│
├───────────────┬──────────────────────────────────────────┤ 40% ▾  00:00:12:04│
│ Media Pool    │ Inspector (when open)                    │ ┌───────────────┐ │
│               │                                          │ │               │ │
├───────────────┴──────────────────────────────────────────┤ │    viewer     │ │
│     ↖ ⇔ ▬  ⤓ ⬓ ⇄  ∩ 🔗  ⚑▾      ⊟ − ──●── +       🔊 ━━● │ │     9:16      │ │
├────────────┬─────────────────────────────────────────────┤ │               │ │
│ 00:00:12:04│ 00:00:00:00    00:00:10:00    00:00:20:00   │ │               │ │
│▌[V1] 🔒 ▣  │ ▛▀▀▀▀▀▀▀▀▜▛▀▀▀▀▀▀▀▀▀▀▀▜▛▀▀▀▀▀▜              │ └───────────────┘ │
│▌[A1] 🔒 M  │ ▛∿∿∿∿∿∿∿∿▜▛∿∿∿∿∿∿∿∿∿∿∿▜▛∿∿∿∿∿▜              │ ├───────▼───────┤ │
│▌ A2  🔒 M  │ ▛∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿▜               │  ⏮ ◀ ■ ▶ ⏭ ⟲     │
├────────────┴─────────────────────────────────────────────┴───────────────────┤
│ ▶║ Tempo                          ✎        🚀                          ⌂   ⚙ │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## 6. Screen 4 — Export page

Reference: `export page with files.png`.

```
┌──────────────────────────────────────────────────────────────────────────────────────┐
│ ▤ Render Settings                 Iceland Vlog │ Edited             ▥ Render Queue │ ⋯ │
├─────────────────────────┬────────────────────────────────────┬───────────────────────┤
│ Render Settings -       │ 30% ▾      Timeline ▾   00:03:42:00│ Render Queue          │
│   YouTube - 1080p       │ IN 01:00:00:00  OUT 01:03:42:00    │ ┌───────────────────┐ │
│  ▭        ▯       ◻     │ ┌────────────────────────────────┐ │ │ ≡ Job 1      ✎  ✕ │ │
│  16:9    9:16     1:1   │ │                                │ │ │ ▶ Iceland Vlog    │ │
│ YouTube  TikTok  Insta  │ │            viewer              │ │ │   ~/Videos/Icel…  │ │
│ ═══════                 │ │                                │ │ │ ━━━━━━━──── 64 %  │ │
│                         │ └────────────────────────────────┘ │ └───────────────────┘ │
│   File Name [Iceland_V] │ ├──────────────▼─────────────────┤ │ ┌───────────────────┐ │
│    Location [~/Videos ] ( Browse )   ⏮ ◀ ■ ▶ ⏭ ⟲     ⇥ ⇤    │ │ ≡ Job 2      ✎  ✕ │ │
│ ─────────────────────── │                                    │ │ ▯ Iceland Vlog    │ │
│  Resolution [1920x1080▾]│        Render ( Entire Timeline ▾ )│ │   ~/Videos/Icel…  │ │
│  Frame rate [Timeline ▾]├────────────┬───────────────────────┤ └───────────────────┘ │
│      Format [MP4      ▾]│ 01:00:23:15│ 01:00:00   01:01:00   │                       │
│ Video Codec [H.264    ▾]│▌ V1  ▣     │ ▛▀▀▀▀▜▛▀▀▀▀▀▀▜▛▀▀▀▜   │                       │
│ Audio Codec [AAC      ▾]│▌ A1  M     │ ▛∿∿∿∿▜▛∿∿∿∿∿∿▜▛∿∿∿▜   │                       │
│  ☐ Chapters from markers│▌ A2  M     │ ▛∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿▜    │                       │
│  ☐ Upload directly to … │            │                       │                       │
│   ( Add to Render Queue)│            │                       │        ( Render All ) │
├─────────────────────────┴────────────┴───────────────────────┴───────────────────────┤
│ ▶║ Tempo    ◔ Rendering 64 %               ✎        🚀                         ⌂   ⚙ │
└──────────────────────────────────────────────────────────────────────────────────────┘
```

### 6.1 Top bar

Same bar as the Edit page, with Resolve's Deliver buttons: `Render Settings` at the left and
`Render Queue` at the right. Each opens or closes its panel. (Resolve's Tape and Clips
buttons are left out.)

### 6.2 Left — Render Settings (360 px, `tempo_bg_surface`)

- **Title row:** "Render Settings - {preset}" in `.tempo-heading` `tempo_text_bright`, on
  `tempo_bg_deep`, updating with the selection — as in Resolve.
- **Format strip:** a horizontally scrolling row of formats. There are no platform icons or
  logos. Each item is a **drawn outline of the frame shape** (a 1.5 px rounded rectangle in
  the real proportions, fitted in a 44 × 44 px box), with the ratio written inside it, the
  size below, and one line saying what it is best for:

  | Shape | Ratio | Size | "Best for" line |
  |---|---|---|---|
  | wide rectangle | 16:9 | 1920 × 1080 | YouTube, Vimeo |
  | wide rectangle | 16:9 | 3840 × 2160 | YouTube 4K |
  | tall rectangle | 9:16 | 1080 × 1920 | TikTok, Reels, Shorts |
  | square | 1:1 | 1080 × 1080 | Instagram feed |
  | slightly tall rectangle | 4:5 | 1080 × 1350 | Instagram portrait |
  | dashed rectangle | — | as set below | Custom |

  The selected item's outline and text are `tempo_text_bright` with a 2 px `tempo_accent`
  line under it; the rest are `tempo_text_dim`. A thin rounded scroll indicator sits under
  the strip. If the chosen ratio differs from the project's, a **Fit / Fill** choice appears
  (black bars, or crop to the centre).
- **File Name** and **Location** rows: label right-aligned in `tempo_text_dim`, field on
  `tempo_bg_field`, and a `Browse` pill button after Location. Free space is shown under
  Location in `.tempo-small`; it turns `tempo_warn` with a warning icon when the estimate
  will not fit.
- A 1 px `tempo_line` rule, then the **settings form**: label right-aligned at 32 % of the
  width, a drop-down on `tempo_bg_field` to its right. Rows: Resolution, Frame rate, Format,
  Video Codec, Quality, Audio Codec. Picking a preset fills them in; a beginner never has to
  touch them.
- **Check boxes** below the form, aligned with the drop-downs:
  - `Chapters from markers`
  - `Upload directly to {platform}` — shown only for presets that have an upload plugin
    installed. This is where Resolve puts its own "Upload directly to TikTok" option.
- **Add to Render Queue:** a pill button at the bottom right of the panel.

### 6.3 Centre — viewer, range, timeline

- The viewer, as on the Edit page, with an extra line under its header showing
  `IN`, `OUT` and `DURATION` in `.tempo-small` (labels `tempo_text_dim`, values `tempo_text`).
- A row with **Render** and a drop-down: Entire Timeline, or In/Out Range.
- A read-only timeline below it: same clips and colours, simpler track headers (name and
  enable or mute only). `I` and `O` set the range; nothing can be edited here.
- Resolve's clip-thumbnail strip between the viewer and the timeline is left out.

### 6.4 Right — Render Queue (360 px, `tempo_bg_surface`)

- **Title row:** "Render Queue" in `.tempo-heading` `tempo_text_bright` on `tempo_bg_deep`.
- **Job card** on `tempo_bg_card`, 1 px `tempo_line`, radius 3:
  - header: drag grip, "Job 1", and at the right an edit pencil and a remove ✕;
  - body: the ratio outline, the project name in `.tempo-heading`, the output path in
    `.tempo-small` `tempo_text_dim`, shortened from the left with `…`.

| State | Shown in the card |
|---|---|
| Waiting | Nothing extra |
| Rendering | A 3 px `tempo_accent` progress line, percent and time left |
| Done | "Completed in 00:01:12" in `tempo_text_dim`, and text buttons `Show in Files`, `Copy chapters`, `Share…` |
| Failed | Warning icon, a one-line reason in `tempo_error`, `Retry` |
| Cancelled | "Cancelled", `Retry` |

- **Render All:** a pill button under the list, right-aligned. While rendering it reads `Stop`.
- Jobs run one at a time.

---

## 7. Shared components

### 7.1 Buttons

Resolve's buttons are outlined pills. Tempo uses the same shape everywhere.

| Kind | Look | Used for |
|---|---|---|
| Pill | Transparent, 1 px `tempo_line_button` outline, `tempo_text` label, 28 px high, 16 px side padding. Hover: `tempo_bg_hover` fill, `tempo_text_bright` | All text buttons |
| Pill, default | Same, with a `tempo_text_bright` label and outline | The default action in a dialog |
| Icon | No outline; `tempo_text_dim`, `tempo_text_bright` on hover or when on | Toolbars, transport |
| Destructive | Pill with a `tempo_error` label | Move to Trash, Remove |

There are no filled, coloured buttons. The accent is reserved for state.

### 7.2 Fields and drop-downs

`tempo_bg_field` fill, 1 px `tempo_line` border, radius 3, 26 px high, `tempo_text`. Drop-downs
have a small chevron at the right. Focus: border becomes `tempo_accent`. Check boxes are
16 px squares on `tempo_bg_field`; ticked, they show a `tempo_text_bright` tick.

### 7.3 Dialogs

`tempo_bg_panel`, radius 10, 16 px padding, title top-left in `.tempo-heading`, pill buttons
bottom-right with the default action last. Dialogs are for decisions only: New Project, Quick
Export, Missing Media, Project Settings, Preferences, Plugins, Marker, Keyboard Shortcuts.

### 7.4 Messages

- **Toasts** for results that need no action ("Imported 4 clips", "Chapters copied"):
  3 seconds, bottom centre above the page bar.
- **Activity line** in the page bar for anything with progress.
- **Banners** at the top of a panel for first-use hints, each with a dismiss button.

### 7.5 Focus

Every control is reachable with Tab. The focus ring is a 1 px `tempo_accent` outline,
2 px outside the control. A panel with keyboard focus shows a 1 px `tempo_accent` line along
its top edge, as Resolve highlights the active panel.

---

## 8. App icon

A rounded square, fill `#212126`, with a play triangle in `tempo_accent` and two short
vertical bars to its right. Flat. Provide a scalable SVG and a single-colour symbolic version.

---

## 9. Accessibility

- Body text meets 4.5:1 contrast against its background.
- Status is never shown by colour alone.
- With the desktop high-contrast setting on: `tempo_text_dim` becomes `tempo_text`,
  `tempo_line_button` becomes `#8a8a92`, and clip outlines become 1 px white.
- Font sizes are relative, so the system text-size setting scales the whole UI.
- The page-bar buttons are icon-only like Resolve's, so each has an accessible name and a
  tooltip ("Edit — Shift+4", "Export — Shift+8").
- Custom-drawn widgets expose a name and role; the timeline exposes the selected clip's name
  and time range.

---

## 10. GTK implementation notes

- Force dark: `adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark)`.
- Load `tempo.css` from a GResource at application priority. Redefine libadwaita's named
  colours (`window_bg_color`, `headerbar_bg_color`, `view_bg_color`, `card_bg_color`,
  `accent_bg_color`, …) from the Tempo tokens so stock widgets pick up the palette.
- Pill buttons: `button { border-radius: 9999px; background: none; border: 1px solid @tempo_line_button; }`
  scoped under a `.tempo` class on the window so dialogs from the system are not affected.
- Use `@define-color`, not CSS custom properties: the project targets GTK 4.14 and `var(--…)`
  needs 4.16.
- The timeline, ruler, scrub bar and meter are single custom widgets drawn in `snapshot()`.
- Symbolic icons only, recoloured by CSS `color`. Ship Tempo's own icon set in the
  GResource so the look does not depend on the system icon theme.

---

## 11. Things not to do

- No second accent colour. No filled coloured buttons.
- No gradients, shadows, glows or blur.
- No platform logos or icons on export formats; show the frame ratio instead.
- No thumbnails on timeline clips.
- No panel open by default unless a beginner needs it in the first five minutes.
- No sample or placeholder clips. An empty timeline is empty.
- No control that is drawn but not connected to a working feature.
