# UI Specification — Tempo

**Version:** 2.1 (2026-10-05)
**Reference:** DaVinci Resolve 20 (screenshots in `EX/DVR_UI/`, key export in `EX/DaVinci Resolve Keys.txt`)
**Toolkit:** GTK 4.14+ and libadwaita 1.6+
**Replaces:** version 1.0 (Cut page, Edit page, Export page with a top switcher).

This document says which widgets build each screen and how they behave.
How they look is in `VISUAL_DESIGN.md`. Keys are in `KEYBINDS.md`.

---

## 1. Screens

Tempo has four screens. Two are start-up screens; two are pages of an open project.

| # | Screen | When | Resolve equivalent |
|---|---|---|---|
| 1 | Loading | App start | Splash |
| 2 | Project Manager | After loading; `Shift+1`; Home button | Project Manager |
| 3 | Edit page | A project is open; `Shift+4` | Edit page |
| 4 | Export page | A project is open; `Shift+8` | Deliver page |

There is no Cut page. Editing happens on the Edit page only.

---

## 2. Window structure

One `AdwApplicationWindow`. Screens are children of stacks, so switching never creates or
destroys a window.

```
AdwApplicationWindow
└── AdwToastOverlay
    └── GtkStack  "root"                       (crossfade off)
        ├── "loading"   LoadingView
        ├── "projects"  ProjectManagerView
        └── "project"   AdwToolbarView
            ├── [top]     AdwHeaderBar          top bar (§5.1)
            ├── [content] GtkStack "pages"
            │   ├── "edit"    EditPage          (§5)
            │   └── "export"  ExportPage        (§7)
            └── [bottom]  PageBar               (§6)
```

- Minimum size 1024 × 640. Default 1366 × 768, or the size saved at last close.
- Window title: `{project name} — Tempo`.
- The viewer is one widget instance, re-parented between the Edit and Export pages. Only one
  viewer is ever decoding.

### 2.1 Rules for every screen

1. **The UI thread never waits.** No file reading, decoding, database writes or network
   calls on the GTK thread. Work runs on a worker and reports back with
   `glib::MainContext::spawn_local` plus an `async-channel`. (`glib::MainContext::channel`
   no longer exists in glib 0.20.)
2. **Every timeline change is a command** sent through `CommandLog`, so it can be undone.
   A slider drag is one command, committed on release.
3. **No placeholder data.** An empty list shows its empty state.
4. **Every control has an accessible name and a tooltip** that states the action and its key.
5. **Lists use recycling widgets** (`GtkGridView`, `GtkListView`, `GtkColumnView`) with a
   `GListModel`. Do not use `GtkFlowBox`, `GtkListBox` or `GtkTreeView` for media or projects.

---

## 3. Loading

`LoadingView`: a centred `GtkBox` with `GtkImage` (icon), `GtkLabel` (name),
`GtkProgressBar`, `GtkLabel` (status).

- Start-up tasks run on a worker thread and send `(step_index, text)` messages.
- When all steps finish, the root stack switches to `projects`, or straight to `project` if
  the app was started with a file path.
- If everything finishes within 300 ms, the view is never shown.
- A fatal error (no usable graphics device) shows the message and a **Quit** button.
- A non-fatal error (no audio device, hardware decode unavailable) does not stop loading. It
  is reported as a toast after the Project Manager appears.

---

## 4. Project Manager

```
GtkBox (vertical)
├── GtkBox  header:  GtkLabel "Projects" · new folder · size GtkScale · sort · grid/list toggles · search
├── GtkScrolledWindow
│   └── GtkGridView  (model: GtkFilterListModel over the project list)
│       or GtkColumnView in list mode
└── GtkCenterBox  footer:  [start] Export · Import      [end] New Project · Open
```

- **Model:** one item per known project: path, name, thumbnail path, resolution, frame rate,
  duration, modified time, missing flag. The list is stored in
  `~/.local/share/tempo/projects.json`; thumbnails in `~/.cache/tempo/project-thumbs/`.
  Opening this screen must not open every project file.
- **Search** filters by name as the user types.
- **Double-click / Enter / Open:** load the project on a worker, then switch to `project`.
  While loading, the card shows a spinner.
- **New Project:** opens the New Project dialog (§8.1).
- **Import:** `GtkFileDialog` filtered to `*.tempo`; adds the file to the list and opens it.
- **Export:** saves a copy of the selected project file to a chosen folder.
- When a project is already open, **Open** reads **Close** and returns to it, as in Resolve.
- The size slider changes the thumbnail size (160–320 px wide).
- **Context menu:** Open, Rename, Duplicate, Show in Files, Move to Trash. Move to Trash uses
  `gio::File::trash` and offers Undo in a toast.
- **Autosave recovery:** if a newer autosave exists for a project, opening it asks
  "Restore unsaved changes from {time}?" with **Restore** and **Discard**.

---

## 5. Edit page

```
GtkBox (horizontal)
├── GtkRevealer → EffectsColumn        only when Effects is open: full-height left column
│       GtkPaned (vertical):  MediaPool (if open)  /  Effects
└── GtkPaned (vertical)                upper/lower split, user-resizable
    ├── [start] GtkBox (horizontal)    upper area
    │   ├── GtkRevealer → MediaPool    (here only when Effects is closed)
    │   ├── ViewerArea                 (hexpand)
    │   └── GtkRevealer → Inspector
    └── [end] GtkBox (vertical)        lower area
        ├── TimelineToolbar
        └── GtkBox (horizontal)
            ├── TimelineWidget         (hexpand, vexpand)
            └── AudioMeter

This reproduces Resolve's behaviour: with only the Media Pool open it sits beside the viewer;
opening Effects puts Effects under the Media Pool in a column that runs down beside the
timeline. The Media Pool widget is re-parented between the two places, not duplicated.
```

For vertical projects the same widgets are arranged as described in
`VISUAL_DESIGN.md §5.10`: an outer horizontal `GtkPaned` holds the normal arrangement
(without the viewer) at the start and the viewer at the end. The arrangement is chosen when
the project loads or its resolution changes.

### 5.1 Top bar (`AdwHeaderBar`)

| Position | Widget | Action |
|---|---|---|
| Start | `GtkToggleButton` "Media Pool" | `win.panel-media-pool` |
| Start | `GtkToggleButton` "Effects" | `win.panel-effects` |
| Title | `GtkBox`: project name · divider · "Edited" when dirty | — |
| End | `GtkButton` "Quick Export" | `win.quick-export` |
| End | `GtkToggleButton` "Inspector" | `win.panel-inspector` |
| End | `GtkMenuButton` main menu `⋯` | see below |

The panel toggles have no keys, as in Resolve. `Ctrl+2`, `Ctrl+6` and `Ctrl+9` move keyboard
focus to the Media Pool, Effects and Inspector, opening the panel first if it is closed
(`KEYBINDS.md §10`).

On the Export page the same bar shows `Render Settings` at the start and `Render Queue` at
the end instead.

**Main menu:** Import Media…, Save, Save As…, Project Settings…, Quick Export…, Undo, Redo,
Preferences…, Plugins…, Keyboard Shortcuts, About Tempo, Quit.

Panel open/closed state, panel widths and the upper/lower split are saved per project.

### 5.2 Media Pool

```
GtkBox (vertical)
├── GtkSearchEntry
├── GtkBox  bin chips            (hidden until the user creates a bin)
└── GtkScrolledWindow → GtkGridView   (list mode: GtkColumnView)
```

- **Import:** `Ctrl+I`, the menu, or dropping files on the panel. Each file is probed on a
  worker. The item appears at once with a placeholder icon; the thumbnail fills in when ready.
  Nothing is converted on import.
- **Item:** thumbnail, name, duration badge. Tooltip: resolution, frame rate, codec.
- **Double-click:** opens the clip in the viewer in source mode.
- **Drag to timeline:** inserts the clip (§5.6).
- **Context menu:** Open in Viewer, Insert, Append to End, Relink…, Remove from Project.
- **Missing media** shows the offline look and offers Relink….
- **Thumbnails** are cached on disk under `~/.cache/tempo/thumbs/`, keyed by path, size and
  modification time. They are not stored in the project file.

### 5.3 Effects

Two parts side by side, as in Resolve: a short category list at the left (Toolbox ▸
**Transitions**, **Titles**, **Filters**) and, at the right, a `GtkListView` of that
category's items in collapsible groups.

- Built in — Transitions: Cross Dissolve, Dip to Colour. Titles: Text, Lower Third.
  Filters: none.
- Fade in and fade out are not list items. They are the fade handles on every clip (§5.6).
- Plugins add items to these groups (`PLUGIN_SPEC.md`).
- Drag a transition onto a cut; drag a title onto a video track; drag a filter onto a clip.
- Double-click applies the item to the selection.

### 5.4 Viewer

```
GtkBox (vertical)
├── GtkCenterBox  header:  [start] zoom GtkDropDown · [center] timeline/clip name · [end] timecode · GtkMenuButton
├── GtkOverlay
│   ├── VideoSurface                    (the picture)
│   └── badges (PROXY, dropped frames)
├── ScrubBar                             (custom widget)
└── GtkCenterBox  transport:  [start] mode menu · [center] ⏮ ◀ ■ ▶ ⏭ ⟲ · [end] Mark In · Mark Out
```

- **VideoSurface** is a `GtkPicture` inside `GtkGraphicsOffload`. Decoded frames arrive as
  `GdkTexture`s (`GdkDmabufTexture` when hardware decode is active, so the desktop compositor
  scales and converts colour; a memory texture otherwise). The widget redraws only when a new
  texture arrives. The rendering path is defined in `MEDIA_ENGINE.md`.
- **Two modes**, one viewer:
  - *Timeline* (default): shows the edited timeline at the playhead.
  - *Source*: shows one Media Pool clip, with its own In/Out. Entered by double-clicking a
    clip. `Q` switches between the two, as in Resolve's single-viewer mode.
- **Dual-viewer mode** (viewer menu, only offered when the window is 1600 px or wider):
  source on the left, timeline on the right, as in Resolve's default.
- **Viewer menu:** Playback Quality (Full, Half, Quarter), Safe Area, Dual Viewer.
- **Transport** buttons call the same actions as the keys.
- **Timecode** updates from a tick callback while playing; it is not polled by a timer.
- **Cinema viewer** (`P` or `Ctrl+F`): the window goes full-screen and shows only the
  picture. `Escape` returns.

### 5.5 Inspector

`GtkScrolledWindow` containing a vertical `GtkBox` of sections. Each section is a header row
(name, reset button) and a `GtkGrid` of rows: label right-aligned in column 0, value widgets
in column 1, per-row reset in column 2. This gives Resolve's Inspector look
(`VISUAL_DESIGN.md §5.4`); libadwaita preference rows are not used here.

`NumberField` is a small custom widget: a `GtkText` that also changes its value on horizontal
drag. It is used for every number in the Inspector.

| Section | Rows | Widget |
|---|---|---|
| Transform | Zoom X/Y (linked), Position X/Y, Rotation | `NumberField`; `GtkScale` for Rotation |
| Composite | Opacity | `GtkScale` + `NumberField` |
| Audio | Volume (dB), Pan | `GtkScale` + `NumberField` |
| Title | Text, Font, Size, Colour, Background, Position | `GtkEntry`, `GtkFontDialogButton`, `NumberField`, `GtkColorDialogButton`, `GtkCheckButton` |
| Transition | Type, Duration, Alignment | `GtkDropDown`, `NumberField` |
| Marker | Name, Colour, Note | `GtkEntry`, `GtkDropDown` |

- Sections shown depend on the selection (`VISUAL_DESIGN.md §5.4`).
- Each group header has a reset button.
- The viewer updates live while a value is dragged. One undo step is recorded on release.
- Position is stored as a fraction of the frame size, so it survives a resolution change. It
  is shown in pixels.
- Several clips selected: rows show the shared value or "Mixed"; an edit applies to all.
- Plugin sections are built by the host from the plugin's declared parameters.

### 5.6 Timeline toolbar and timeline

**Toolbar** (`GtkBox`): three linked `GtkToggleButton`s for the edit mode (one group), three
`GtkButton`s for Insert, Overwrite, Replace, two `GtkToggleButton`s for Snapping and Linked
Selection, one split button for Add Marker (click adds; the arrow picks the colour), a Zoom
to Fit button, a zoom `GtkScale` with − and + buttons, and a volume `GtkScale`.

**TimelineWidget** is one custom widget (a `GtkWidget` subclass) that draws the ruler, track
headers, tracks, clips, markers and playhead in `snapshot()` using GTK's own render nodes.
It is not built from buttons and boxes.

- It implements `GtkScrollable`, so it sits in a `GtkScrolledWindow` and gets scrollbars and
  kinetic scrolling for free.
- It draws only what is visible.
- It keeps a retained layout (clip rectangles) rebuilt only when the timeline, zoom or size
  changes. Moving the playhead redraws only the playhead.
- Track header controls (lock, enable/mute) are real `GtkToggleButton` children placed by the
  widget, so they are keyboard- and screen-reader-accessible.

**Destination.** The outlined name box on a track header marks where `F9`–`F12` edits and
pastes go. Click a track's name box to make it the destination. One video and one audio
track are destinations at a time; `V1` and `A1` by default.

**Tracks.** A new project has `V1`, `V2`, `A1`, `A2`. Up to four video and four audio tracks.
Add or delete a track from the track header's context menu, as in Resolve.

**Pointer behaviour**

| Mode | Action | Result |
|---|---|---|
| Selection (`A`) | Click a clip | Select it. `Ctrl+click` adds; drag on empty space draws a selection box |
| | Drag a clip | Move it; another track if dragged up or down. Overwrites what it lands on |
| | Drag a clip edge | Trim that edge; leaves a gap |
| | Drag a fade handle | Set fade in or fade out length |
| | `Alt+drag` a clip | Duplicate it |
| Trim (`T`) | Drag a clip edge | Ripple trim: later clips move to close or make room |
| | Drag a cut between two clips | Roll: move the cut, total length unchanged |
| Blade (`B`) | Click a clip | Split it at that point |
| Any | Click or drag in the ruler | Move the playhead (scrub) |
| Any | Double-click a marker | Open the Marker dialog (name, colour, note) |
| Any | Right-click | Context menu for clip, track, marker or empty space |

- **Snapping** (when on): clip edges, the playhead, markers and In/Out attract a dragged
  edge within 8 px.
- **Scrubbing** shows the nearest keyframe while dragging fast and the exact frame as soon as
  the pointer pauses or is released.
- **Drop from Media Pool:** a ghost clip shows the landing place. Dropping on empty space
  places the clip; dropping on a clip overwrites; holding `Ctrl` inserts and ripples. Video
  with sound places a linked video clip and audio clip.
- **Drop files from the file manager** onto the timeline: import, then place.
- **Locked track:** nothing on it can be selected or changed.

**Clip context menu:** Cut, Copy, Paste, Delete, Ripple Delete, Split at Playhead,
Enable Clip, Link Clips, plus any plugin commands.

### 5.7 Audio meter

Custom widget, two bars. Fed by peak values the audio thread writes to atomics; drawn from a
tick callback only while audio is playing.

---

## 6. Page bar

```
GtkCenterBox
├── [start]  Tempo icon and name · ActivityButton (one-line status; opens a GtkPopover of running jobs)
├── [center] GtkBox:   GtkToggleButton Edit · GtkToggleButton Export      (icon-only, 150 px each)
└── [end]    GtkButton Home (Project Manager) · GtkButton Settings
```

- The two page buttons are one toggle group.
- **Activity** shows the most important running job: export, then proxy generation, then
  waveform or thumbnail work. Idle states "Saved" and "Autosaved" show for 3 seconds and fade.
- The popover lists each job with a progress bar and a cancel button.
- **Home** saves the project, closes it and returns to the Project Manager. If an export is
  running it asks first.

---

## 7. Export page

```
GtkBox (horizontal)
├── GtkRevealer → RenderSettings   (360 px, GtkScrolledWindow)
├── GtkBox (vertical, hexpand)
│   ├── Viewer                      (shared instance, timeline mode only)
│   ├── GtkDropDown  "Render: Entire Timeline | In/Out Range"
│   └── TimelineWidget              (read-only mode)
└── GtkRevealer → RenderQueue      (360 px)
```

### 7.1 Render settings

| Row | Widget | Notes |
|---|---|---|
| Format | Horizontal `GtkScrolledWindow` of toggle items. Each draws the frame shape for its ratio, then ratio, size and a "best for" line. No platform icons | Small fixed set |
| File Name | `GtkEntry` | Defaults to the project name |
| Location | `GtkEntry` + `Browse` button | `GtkFileDialog::select_folder`; free space shown below |
| Resolution, Frame rate, Format, Video Codec, Quality, Audio Codec | `GtkDropDown` in a two-column `GtkGrid`, labels right-aligned | Filled in by the preset |
| Chapters from markers | `GtkCheckButton` | §7.4 |
| Upload directly to {platform} | `GtkCheckButton` | Only when an upload plugin provides that platform; §7.5 |
| Add to Render Queue | `GtkButton` | `Ctrl+Return` |

The form is a plain grid with right-aligned labels, as in Resolve's Render Settings, not a
libadwaita preferences list.

- Picking a preset fills the Video and Audio rows. Changing any of them selects **Custom**.
- Built-in presets:

| Format | Size | Best for | Video | Audio | File |
|---|---|---|---|---|---|
| 16:9 | 1920 × 1080 | YouTube, Vimeo | H.264, high quality | AAC 192k | MP4 |
| 16:9 4K | 3840 × 2160 | YouTube 4K | H.264, high quality | AAC 192k | MP4 |
| 9:16 | 1080 × 1920 | TikTok, Reels, Shorts | H.264, high quality | AAC 192k | MP4 |
| 1:1 | 1080 × 1080 | Instagram feed | H.264, high quality | AAC 192k | MP4 |
| 4:5 | 1080 × 1350 | Instagram portrait | H.264, high quality | AAC 192k | MP4 |
| Custom | as set | — | as set | as set | as set |

- If a preset's shape differs from the project's (for example Vertical on a 16:9 project), a
  row appears: **Fit** (black bars) or **Fill** (crop to centre).
- The hardware encoder is used when available; the user does not choose it.
- User presets are saved to `~/.config/tempo/export_presets.json`.

### 7.2 Render queue

`GtkListView` of job cards plus a **Render All** button (`Ctrl+Shift+Return`).

- Adding a job takes a snapshot of the timeline, so later edits do not change a queued job.
- Jobs run one at a time on the export thread. Editing continues on the Edit page.
- States and their controls are in `VISUAL_DESIGN.md §6.3`.
- A finished job offers **Show in Files**, **Copy chapters** and **Share…**.
- Cancel stops the encoder and deletes the partial file.
- The queue is kept in memory only and is cleared when the app closes. Closing with a job
  running asks for confirmation.
- There is no pause. Cancel and Retry cover the need with less to go wrong.

### 7.3 Quick Export

`Ctrl+Shift+E` or the main menu, from either page. An `AdwDialog` with the preset cards, file
name, location and an **Export** button. It adds one job and starts it. Progress shows in the
activity area, so the user can keep editing.

### 7.4 Chapters from markers

Markers that have a name become chapters.

- **In the file:** when the switch is on, chapters are written into the MP4 or MKV, so video
  players show them.
- **As text:** **Copy chapters** (job card, or `Ctrl+Alt+M`) copies a list in the form
  video sites read from a description:

```
00:00 Intro
01:25 Setting up the tent
04:10 Night shots
```

- Times are relative to the start of the exported range. Markers outside the range are left out.
- Before copying, Tempo checks YouTube's rules and explains any problem in a dialog with a
  **Copy anyway** button:
  - the first chapter must be at 00:00 (Tempo offers to add "Intro" at 00:00);
  - there must be at least three chapters;
  - each chapter must be at least 10 seconds long.
- Unnamed markers are editing notes only and are never exported.

### 7.5 Share

**Share…** on a finished job opens a `GtkPopover` listing upload targets.

- **Without an upload plugin:** the list holds "Open YouTube upload page", "Open Instagram",
  "Open TikTok upload page". Each opens the site in the browser and shows the exported file in
  the file manager, ready to drag in.
- **With an upload plugin** (`PLUGIN_SPEC.md §7`): the list also shows each connected account.
  Choosing one opens a small form (title, description with the chapter list pre-filled,
  visibility) and uploads in the background with progress in the activity area.
- The same plugin makes the **Upload directly to {platform}** check box appear in Render
  Settings for matching presets — the place Resolve puts its own "Upload directly to TikTok"
  option. With it ticked, the upload form opens as soon as the job finishes.
- Direct upload is not part of the core. Each platform needs its own developer approval and
  its rules change often, so it ships as plugins after 1.0.

---

## 8. Dialogs

All are `AdwDialog` unless noted.

### 8.1 New Project

| Row | Widget | Default |
|---|---|---|
| Name | `AdwEntryRow` | "Untitled Project" |
| Location | folder row | `~/Videos/Tempo` |
| Shape | `AdwComboRow` | Landscape 16:9, Vertical 9:16, Square 1:1 |
| Resolution | `AdwComboRow` | 1080p |
| Frame rate | `AdwComboRow` | 30 |

Buttons: **Cancel**, **Create**. If the first imported clip has a different size or frame
rate, a banner asks "Match project to this clip?" — the same question Resolve asks.

### 8.1a Marker

Opened by `Ctrl+M`, `Shift+M` or double-clicking a marker. Rows: Time (timecode), Name,
Note, and a row of seven colour dots. Buttons: **Remove Marker** (left), **Done** (right).
A hint under Name reads "Named markers become chapters when you export."

### 8.2 Missing Media

Lists the files that were not found. **Locate Folder…** searches a chosen folder for matching
names and durations; **Skip** opens the project with those clips offline.

### 8.3 Project Settings (`Shift+9`)

Resolution, frame rate (locked once the timeline has clips), proxy folder.

### 8.4 Preferences (`AdwPreferencesDialog`, `Ctrl+,`)

| Page | Settings |
|---|---|
| General | Font scale, autosave interval (1, 2, 5, 10 minutes), reset first-use hints |
| Playback | Default quality, hardware decoding on/off, automatic proxies on/off |
| Storage | Frame cache size, cache folder, **Clear cache** with size shown |

### 8.5 Plugins (`AdwPreferencesDialog`)

One `AdwExpanderRow` per plugin: name, version, enable switch; expanded: description, what it
adds, permissions, **Remove**. An **Install…** button opens a file dialog for `.tempo-plugin`
files; dropping a file on the dialog also installs it.

### 8.6 Keyboard Shortcuts (`F1`)

Built from `keybinds.json`. Groups follow `KEYBINDS.md`. Tempo-only keys are under their own
heading.

---

## 9. First-use hints

`AdwBanner` at the top of the relevant panel, each with a **Got it** button. State is saved in
`~/.config/tempo/hints.json`.

| Where | Text |
|---|---|
| Media Pool, empty | "Drop your videos here, or press Ctrl+I." |
| Timeline, empty | "Drag a clip down here to start." |
| Timeline, first clip | "Space plays. B is the blade. Shift+Backspace deletes and closes the gap." |
| Export page, first visit | "Pick a preset, add it to the queue, then Render All." |
| First marker | "Give a marker a name and it becomes a chapter when you export." |

---

## 10. Drag and drop summary

| From | To | Result |
|---|---|---|
| File manager | Media Pool | Import |
| File manager | Timeline | Import and place |
| Media Pool | Timeline | Place (overwrite); `Ctrl` inserts |
| Media Pool | Viewer | Open in source mode |
| Viewer (source mode) | Timeline | Place the In–Out range |
| Effects: transition | Cut between clips | Apply transition |
| Effects: title | Video track | Add title clip |
| Effects: filter | Clip | Apply filter |
| Timeline clip | Timeline | Move |
| `.tempo-plugin` file | Plugins dialog | Install |
