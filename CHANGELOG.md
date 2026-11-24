# Changelog

All notable changes to Tempo are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Project Manager & File Operations (DaVinci Resolve parity):**
  - Hierarchical Project Manager folders (`New Folder`, `Rename…`, `Delete Folder` returning projects to root, `Move to Folder…`, breadcrumb navigation), Grid/List views (`GtkColumnView`), Sort dropdown (`Date Modified`, `Name A–Z`, `Name Z–A`), Thumbnail Size slider (`160–320 px`), real project card thumbnails (`~/.cache/tempo/project-thumbs/`), and footer `Export` button.
  - `New Project` dialog now validates project names case-insensitively (`"Name Already Exists"` modal) and lets users pick a custom save folder.
  - `Save Project As…` (`Ctrl+Shift+S` / `app.save-as`) and `Project Settings…` (`Shift+9` / `win.project-settings`, also accessible from the bottom-right Settings button).
- **Media Pool Bins, Usage Bar & Hover-Scrubbing:**
  - `New Bin` (`Ctrl+Shift+N` / `win.new-bin`) + `Master` bin hierarchy, collapsible Bin Tree sidebar, breadcrumb navigation (`Master / …`), drag-and-drop for clips and timelines across bins, active-bin inheritance on `Create New Timeline`, recursive `Import Folder…`, Grid/List view toggle, Sort selector, and Thumbnail Zoom slider (`64–160 px`).
  - Red bottom `"Used in Timeline"` indicator bar on placed clips, debounced thumbnail hover-scrubbing into the Source Viewer, and SQLite schema migration **v6 → v7** persisting `timelines.bin_id`.
- **Timeline Editing, Resolve Keybinds & Viewer Controls:**
  - Track header right-click menu (`Add Video Track`, `Add Audio Track (Stereo/Mono)`, `Delete Track` via undoable `DeleteTrackCommand`), `Shift+Scroll` track height resize (`36–140 px`), `Alt+drag` atomic clip duplication (`CompositeCommand`), and Effects drop support for `Filters`, `Titles`, and `Audio Transitions > Crossfade`.
  - Split Clip (`Ctrl+\`) falls back to unlocked clips under the playhead when nothing is selected; `M` reopens `Edit Marker` when already on a marker; `J`/`L` & `Shift+J`/`Shift+L` shuttle up to `64×`.
  - Viewer `Loop` button (`Ctrl+/`), Zoom dropdown (`Fit`, `25%`, `50%`, `100%`, `200%`, `Z`, `Alt+Shift+Z`), Full-Page Viewer (`Shift+F`), Source/Timeline viewer focus (`Ctrl+3` / `Ctrl+5`), and remaining Resolve keybinds (`V` select edit point, `Alt+\` join clip, `Y`/`Alt+Y`/`Ctrl+Y`/`Ctrl+Alt+Y` track selection, `Ctrl+Shift+V` paste insert, `Shift+F10` ripple overwrite, `Ctrl+Shift+,`/`.` swap, `±1`/`±3 dB` volume steps, `Ctrl+Shift+1..4` video track toggles, `Alt+L` play again, `Alt+/` play In to Out, `/` play around, `Shift+S` scrub audio, `;`/`'` first/last frame, `Shift+A` mark selection).
- **Inspector Multi-Clip Editing & Effects Panel:**
  - Multi-clip batch editing across `Transform`, `Composite`, `Audio`, `Fades`, and `Title` with `"Mixed"` badges and single-step `CompositeCommand` undo/redo.
  - Full `TitleData` controls (`Font` entry + presets, `Color`, and transparent/color `Background`), `Audio Transitions > Crossfade`, and drag-and-drop sources on `Titles` and `Filters`.
- **Deliver / Export Queue & Custom Controls:**
  - Exposed `Format` (`MP4`, `MOV`, `MKV`), `Video Codec` (`H.264`, `H.265`), `Audio Codec` (`AAC`, `FLAC`, `PCM`), `Normalize` (`Off`, `YouTube (-14 LUFS)`), and `Custom` preset `Width`/`Height` spinbuttons alongside `YouTube 1080p` / `YouTube 2160p` preset labels.
  - Render Queue job card selection toggles `Render 1` (render selected queued job) vs `Render All` when deselected.

## [0.3.0] - 2026-11-20

Alpha. The Edit page works much more like DaVinci Resolve's:
- transitions are their own objects, with Resolve's five dissolves and the full Inspector;
- Resolve-style copy and paste of transitions, and `Up`/`Down` between edits;
- timelines in the Media Pool, and a Create New Timeline dialog that matches Resolve;
- marker colours, zoom presets and upload fields on the Export page;
- end-to-end tests of the real app on a private Wayland session.

### Added

- **Transitions work as in DaVinci Resolve.** A transition is its own object on the timeline.
  - The Effects panel lists Video Transitions › Dissolve, with Resolve's names: **Additive
    Dissolve**, **Blur Dissolve**, **Cross Dissolve**, **Dip To Color Dissolve** (black or
    white for a fade to black or white) and **Non-Additive Dissolve**. The Inspector's
    Transition Type lists the same five. Additive mixes as screen and Non-Additive keeps the
    brighter pixel; Blur blurs both clips up to the middle of the mix. Playback and export
    match. Drag one onto a cut, or onto the start or end of a clip. A box shows
    where it will land. Audio tracks refuse video transitions.
  - On a cut it is centred when both clips have footage to spare; otherwise it starts or
    ends on the cut. On the first clip's start it fades from black, and on the last clip's
    end it fades to black.
  - Click a transition to select it (red outline). Drag its free edge to change its length
    in whole frames; a centred one changes on both sides. A tooltip shows the change and the
    new length. Lengths never go past the footage the clips have.
  - The Inspector shows, in Resolve's order: Transition Type; Duration in seconds and
    frames; **Set as Default Duration** (new transitions from drops and `Ctrl+T` get that
    length); Alignment (Start, Center, End on Edit); **Start Ratio** and **End Ratio**;
    **Ease** (None, In, Out, In & Out); **Transition Curve**; and, for Dip To Color, the
    colour. Ratios and the curve have sliders and reset arrows.
  - Transition Curve shows the curve at the playhead. Typing a value (or dragging its
    slider) keys the curve there, and the mix follows a line through the keys. Ease is
    off while the curve has keys.
  - Playback and export both follow the ratios, ease and curve. Sound cross fades and a
    clip's own fades stay straight.
  - Click a clip edge to select the edit point (green bracket). `Up` and `Down` move the
    selected edit point with the playhead. They count from the timeline playhead, also while
    a source clip is shown in the viewer.
  - `Ctrl+C` on a selected transition copies just the transition, and `Ctrl+V` puts it on the
    selected edit point. So `Up`, `Ctrl+V`, `Up`, `Ctrl+V`… gives every cut the same transition.
  - `Backspace` or `Delete` on a selected transition removes only the transition.
  - `Ctrl+T` adds the standard transition (1 s) on the selected edit, picture and sound;
    `Alt+T` adds it to the picture only and `Shift+T` to the sound only (a Cross Fade).
  - Right-click a transition for Copy, Delete and Change Transition Duration….
  - Sound cross fades are equal power. Video transitions leave the sound alone.
  - Project files now use schema 5. Older dissolves open as transitions.
- **Several timelines per project.**
  - `Ctrl+N` opens Create New Timeline, with a name and the number of video and audio tracks.
  - The Media Pool right-click menu has Create New Timeline too.
  - The name above the viewer is a menu of the project's timelines, sorted by name. Pick one to
    open it. The playhead goes to the start.
  - Each timeline keeps its own clips, markers and undo history.
  - Project files now use schema 3. Older files open as one timeline named "Timeline 1".
- **Edit a marker** by double-clicking it in the timeline ruler. The Markers dialog sets its
  time, duration (as a timecode), name, notes, **keyword** and colour, and removes it, as in
  Resolve. New markers are named "Marker N" with the name selected, so `Ctrl+M`, type,
  `Enter` names it. `Esc` keeps the marker.
- **Markers on the ruler**: hover one to see its time and name; drag it to move it (it snaps
  to cuts and the playhead). A marker with a duration shows as a bar.
- **Marker overlay on the viewer**: with the playhead on a marker, the viewer shows its time
  and name, as in Resolve.
- **Zoom presets** above the timeline: Full Extent, Detail (around the playhead) and Custom
  (the zoom you last set by hand).
- Dragging a clip shows how far it moved, and trimming shows how much, as `ss:ff`.
- **Timelines in the Media Pool.** Double-click to open; right-click to rename, duplicate or
  delete one.
- Create New Timeline has **Empty Timeline**. Turn it off to start the timeline with the clip
  selected in the Media Pool.
- Create New Timeline has **Start Timecode** (01:00:00:00 by default), **Audio Track Type**
  (Mono or Stereo), **Use Selected Mark In/Out** (place only the marked part of the clip) and,
  bottom left next to Cancel and Create as in Resolve, **Use Project Settings**. Turn that off to give the timeline its own
  width, height and frame rate. Timecode, frame steps, snapping, transition limits,
  playback and export of that timeline use its own rate. The ruler, viewer, timecode label,
  marker times and EDL export count from the Start Timecode.
- Audio track headers show "2.0" (stereo) or "1.0" (mono). A mono track plays down the
  middle, in playback and export.
- **Ghost tracks**: dragging a clip from the Media Pool into the space above the top video
  track shows a new V2 (and A2 for clips with sound) with headers. Drop there to make the
  tracks and place the clip in one undo step. An audio-only clip dragged below the last
  audio track makes a new audio track. A box shows where the clip will land, the drag shows
  the clip's thumbnail, and the first drop on an empty timeline zooms to fit.
- **Resolve's 16 marker colours** in the Markers dialog (Blue, Cyan, Green, Yellow, Red,
  Pink, Purple, Fuchsia, Rose, Lavender, Sky, Mint, Lemon, Sand, Cocoa, Cream), on the ruler
  and on the viewer. A point marker's Duration reads 00:00:00:01, as in Resolve.
- The Inspector's transition page has a **Video** (or **Audio**) header with an on/off
  switch. Off, the transition stays on the timeline (dimmed) but plays and exports as a cut.
- **Transition Curve graph** in the Inspector: click to add a key, drag a key to move it,
  each drag is one undo step. The graph draws the curve as played, with dashed Start and End
  Ratio guides, and the dragged key stays selected.
- The transition edge tooltip shows how far the dragged edge moved (`+` is right) and then
  the new length, as in Resolve.
- The drop box is blue with the clip's thumbnail for video and green for audio.
- **End-to-end tests on Wayland.** `scripts/e2e.sh` runs the real app on a private headless
  mutter with real mouse and keyboard input and screenshots, and never touches your own
  desktop. Any person or AI can add a scenario; see `docs/TESTING_E2E.md`. Scenarios also
  record the app's sound from a private PipeWire and check the Deliver upload fields without
  uploading anything.
- Project files now use schema 6.
- **Edit animations**: trims, deletes and ripples slide clips, transitions, the playhead and
  the view into place over about 120 ms instead of jumping.
- **Upload from the Export page**, as Resolve's "Upload directly to YouTube": when an uploader
  plugin is on, Render Settings has Title, Description, Upload Thumbnail, Privacy and Category.
  They are kept per job, and the job uploads once it has rendered. Plugins receive the
  privacy, category and thumbnail.
- Export render-queue jobs are named "<project> | <timeline>". Each job renders the timeline
  that was open when it was queued.

### Changed

- New transitions are one second long, Resolve's standard transition duration. This was half
  a second.
- Right-click Fade In and Fade Out are one second long, like the Effects panel.
- `Shift+[` and `Shift+]` (Trim Start / Trim End) with nothing selected trim the clip under
  the playhead, as in Resolve.

### Fixed

- A dissolve no longer stops working when the clip before it moves.
- An export whose range starts or ends inside a fade or a Dip To Color keeps the ramp. Audio
  fades are smooth in exports.
- Two sound transitions on one short clip no longer get their handle footage mixed up.
- `Ctrl+T` on a short clip fits its two transitions together.
- A new timeline shows in the Media Pool at once, and the viewer and timecode show its start
  straight away.
- The icon column of the Effects panel items stretched across half the row.
- Fedora 45 (FFmpeg 9):
  - Export failed for titles containing `%`.
  - Sound was slightly wrong just after a seek, because AAC and Opus need a short pre-roll.
  - The export progress bar could step backwards.

## [0.2.0] - 2026-10-21

Alpha. Built for slower computers first:
- playback copes with heavy footage;
- proxies are faster and include sound;
- backups are kept the way Resolve keeps them;
- a set of Resolve Edit page tools beginners reach for: clip speed, fade handles, Solo,
  matching the project to the first clip, and a 10 MB export for Discord.

### Added

- **Change Clip Speed** on `R`, as in DaVinci Resolve. You set a speed from 25 % to 400 %.
  - With **Ripple Sequence** on (the default), the clip changes length and later clips move
    with it.
  - With it off, the clip keeps its length and never covers the next clip.
  - The linked sound changes with the picture.
  - The sound's pitch moves with the speed, in preview and in export.
  - Retimed clips show their speed after their name.
- **Fade handles** on timeline clips: hover a clip and drag a top corner toward the middle.
  Fades snap to whole frames and are drawn as ramps.
- **Solo** (`S`) on audio track headers. It is for listening only: export still includes every
  enabled track.
- **Change Project Frame Rate?** When the first video goes into an empty project, Tempo
  offers to match the project to the clip's size and frame rate. Odd rates snap to the
  nearest standard one, and portrait clips get a vertical project.
- **Project Backups**, kept like Resolve's: every 10 minutes while you edit, 6 from the last
  hour, one per hour for today and one per day for 5 days.
  - Each backup is checked before it is kept.
  - Right-click a project and choose **Project Backups…** to open one as a copy. The
    original is never changed.
  - A project written by an older Tempo is copied to `<file>.v<N>.bak` before it is upgraded.
- A **≤ 10 MB** export card for Discord and email. It picks the bitrate (and 720p when
  needed) so the file fits, and is greyed out when the export is too long to fit watchably.
- Shortcut hints in the bottom bar for what is under the pointer, read from the keybind
  table.
- `scripts/bench_playback.sh` and `docs/PERF.md`: a playback benchmark and its results,
  including hardware (VA-API) playback, proxies and export.
- `CONTRIBUTING.md`: what Tempo accepts and how to contribute. Weak computers and beginners
  come first, the layout and keys stay DaVinci Resolve's, and effects and titles come as
  plugins.

### Changed

- **Project files move to schema 2**, so clips with a changed speed can be saved. A project
  from 0.1.x is copied to `<file>.v1.bak` the first time 0.2.0 opens it. Tempo 0.1.x can't
  open a project once 0.2.0 has saved it; keep the `.v1.bak` if you need to go back.
- Playback decodes more cheaply (no deblocking, half height) while it can't keep up, and
  returns to full quality when it stops or seeks. On a 4K60 stress clip, dropped frames
  fell from 83 % to 9.5 %.
- Moving the playhead no longer redraws the whole timeline. Redraws during playback fell
  from about 30 per second to none.
- Decoded frames are shared with the viewer instead of copied (CPU at Full preview
  quality 52 → 45 %).
- Proxies:
  - They are made only for footage that is on the timeline.
  - They are made entirely on the graphics chip when it can (VA-API decode, scale and
    encode: 5× faster on a 4K clip, at a third of the CPU), falling back to the processor.
  - They now carry the clip's sound, so playback reads one small file.
- At start-up Tempo checks whether hardware decoding and encoding really work. If saved
  settings ask for something the machine can't do, those settings are switched off and
  you are told once.
- Titles are beige on the timeline, and clips with adjustments are a darker shade, as in
  Resolve.
- Choosing a tall format (9:16 or 4:5) on a landscape timeline defaults to Fill.
- Track header buttons are smaller and left-aligned.

### Fixed

- A failed undo could leave the undo history out of step with the timeline.
- A project whose clip settings could not be read opened with those settings reset, and
  the next save made the loss permanent. Such a project now refuses to open and points to
  its backups.
- A proxy marked ready whose file was gone was never rebuilt.
- Playback could show a frame from before a clip's In point. The frame grid also no
  longer drifts at 29.97 and 59.94 fps.
- With hardware decoding on, playing past the end of a file failed to show a picture.
- An export with a size limit failed if hardware encoding was switched on.

### Removed

- Three files in `tempo-audio` that were never compiled.

## [0.1.1] - 2026-10-06

Alpha. Fixes for the problems found in the first hands-on test of 0.1.0, and for more of
the same kind found by reading the code afterwards.

### Added

- Right-click menus on timeline clips (cut, copy, paste, split, delete, unlink, turn off,
  fades), on gaps (Close Gap) and in the Media Pool (insert, overwrite, add to end, open,
  show in Files, remove, import).
- An empty stretch between clips can be clicked and deleted to close it.
- A **Preview** button above the viewer: Full (original file), Half or Quarter. The
  choice is remembered. Half is the default.
- A **Back to Timeline** button while a Media Pool clip is in the viewer.
- Keys from DaVinci Resolve that were documented but not bound: `Shift+L`, `Shift+J`
  (step the speed up and down), `Shift+K` (half speed), `Ctrl+Shift+X` (cut and close the
  gap), `Shift+V` (select the clip at the playhead), `Ctrl+Shift+[` and `Ctrl+Shift+]`
  (trim to the playhead and close the gap), `Alt+Shift+D` and `Alt+Shift+G` (fade to the
  playhead), `Ctrl+6` (Effects). `Ctrl+Shift+E` now starts a Quick Export.
- In and Out marks are drawn as flags in the timeline ruler and on the viewer's scrub bar.
- Clips whose file is missing are marked "Media offline" on the timeline, and
  **Relink Missing Media…** is in the menu.
- A small mark on clips that are linked to another clip.

### Changed

- Export renders a long timeline in sections and joins them. Memory use no longer grows
  with the number of cuts (measured: 574 MB instead of 990 MB for 20 cuts, and flat from
  there). Export also runs at a lower priority, so the computer stays usable.
- Making a proxy uses about a third of a four-thread processor instead of nearly all of
  it, and waits while the timeline is playing or an export is running.
- Quick Export opens the Export page, so the job and its progress are in view.
- Insert (`F9`) and Delete-and-close-gap move every unlocked track, not only the track
  the clip is on, so later clips keep their picture and sound together. When that is not
  possible the gap is kept and a message says why.
- A clip dropped on a track replaces only what is on that track; its other half goes to
  a track that is free there.
- A new project's shape is used as the starting export format.
- A linked clip can be dragged to another track.
- The timeline follows the playhead when a key moves it out of view.

### Fixed

- Blade, split, trim to playhead, cut, copy and paste now act on a clip together with its
  linked sound. After a cut, the two later halves are a linked pair of their own.
- A cut no longer leaves a fade in and a fade out on both halves.
- After double-clicking a Media Pool clip, edits happened at the source clip's position
  instead of at the timeline playhead.
- Clips drew over the track headers when the timeline was scrolled or zoomed.
- Linked clips could not be moved or trimmed by small amounts with snapping on, because
  they snapped to their own old position. A click with a slightly unsteady hand no longer
  moves a clip.
- Short clips could only be trimmed, never moved.
- A clip's end could be dragged past the end of its footage; dragging past a limit threw
  the whole trim away instead of stopping at the limit. Titles and pictures can now be
  extended at their start.
- The timecode read one frame low after stepping by frames.
- Export: picture and sound drifted apart when cuts were not on frame boundaries.
- Redo failed after undoing an edit to the second half of an overwritten clip.
- The player kept reading the original file after a proxy became ready.
- Keyboard shortcuts no longer act on the timeline while a dialog or menu is open, and
  arrow keys work in lists again.
- Closing the window during an export discarded unsaved changes. A project that could not
  be saved could not be closed.
- Opening a project whose file was gone created an empty file in its place.
- The Project Manager's right-click menu acted on the last clicked project, not the one
  under the pointer.
- A double click on Render All started and at once stopped the export.
- The Media Pool forgot the selected clip whenever another file finished importing.
- A cross dissolve kept playing after the clip before it was moved or deleted.
- Track locks were ignored by trims, fades and the Inspector.
- "Restore unsaved changes?" was asked on every opening after choosing Discard.

## [0.1.0] - 2026-10-05

The first release of the Rust rewrite. It replaces the earlier Python prototype.

### Added

- Project Manager: create, open and import projects; search; remove from the list.
- Edit page arranged like DaVinci Resolve's: Media Pool, viewer, Inspector, toolbar,
  timeline, and a page bar at the bottom.
- Timeline with video and audio tracks, markers, snapping, zoom, and a playhead.
- Editing: insert, overwrite, replace, append, place on top, move, trim, ripple trim, blade,
  razor, split, delete, ripple delete, nudge, move between tracks, copy, cut, paste, and
  turning a clip off and on.
- Waveforms on audio clips, worked out in the background and kept on disk.
- An output level meter beside the timeline.
- Cross dissolve between two clips (`Ctrl+T`), for picture and sound.
- Titles (Text and Lower Third) and fade in / fade out, in the viewer and in export, with
  an Effects panel to add them and Inspector sections to edit them.
- Selecting several clips (Ctrl+click, Select All); a clip and its sound are linked and
  move, trim and delete together; rolling a cut with Shift in Trim mode.
- Dual viewer, and a layout for vertical projects with the viewer in a tall column.
- Filters from plugins (Black and White, Saturation, Brightness, Contrast, Blur, Warm come
  built in): added from the Effects panel, adjusted in the Inspector, shown in the viewer
  and in export.
- Upload targets from plugins, listed under Share on a finished export, limited to the
  sites the plugin names.
- Inspector: drag left or right on a setting's name to change it live; the drag is one undo
  step.
- Project Manager: rename, duplicate, show in Files, move to Trash.
- Preferences: autosave interval, playback quality, automatic proxies, cache.
- Lua plugins: sandboxed scripts that add commands to the menu and change the timeline as
  one undo step. A Remove Silence plugin is built in.
- Optional hardware (VA-API) decoding and encoding, off by default, falling back to the
  processor when the graphics driver cannot be used.
- A dialog to find media files that have moved.
- A one-time hint for first-time users.
- Undo and redo for every edit, marker and track change.
- Viewer with a source mode for previewing Media Pool clips and marking In and Out.
- Preview sound through PipeWire. The picture follows the audio clock.
- Inspector: zoom, position, rotation, opacity, volume and pan.
- Background 540p proxies for footage above 720p or not in H.264. Export always uses the
  original files.
- Markers with name, colour and note. Named markers become chapters in the exported file
  and can be copied as a timestamp list, with YouTube's chapter rules checked.
- Export page: outputs chosen by frame shape (16:9, 16:9 4K, 9:16, 1:1, 4:5), a render
  queue, Fit or Fill when the shape differs, and Quick Export.
- Keyboard shortcuts matching DaVinci Resolve 20, checked by a test against a key file
  exported from Resolve.
- Autosave every two minutes, and an offer to restore unsaved work after a crash.
- Its own icon set, so the interface looks the same under any system icon theme.
- AppImage, Flatpak, `.rpm` and `.deb` packages with SHA-256 checksums. The release workflow
  installs each one and starts Tempo from it before publishing.

### Changed

- Decoding runs on a background thread; the interface never waits for it.
- Nothing is decoded while paused.
- Video is decoded with frame threading and scaled straight to the preview size.
- Scrubbing shows keyframes while dragging and the exact frame when the pointer rests.
- The viewer is drawn by GTK on the graphics chip, replacing a path that copied each frame
  to the graphics chip and back.
- Export is done by FFmpeg from the original files, replacing a path that produced blank
  frames.

### Removed

- The Cut page; editing happens on the Edit page.
- The wgpu renderer, the WebAssembly plugin host and the Python helper, none of which the
  editor used any more.

[Unreleased]: https://github.com/MSpider3/Tempo/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/MSpider3/Tempo/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/MSpider3/Tempo/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/MSpider3/Tempo/releases/tag/v0.1.0
