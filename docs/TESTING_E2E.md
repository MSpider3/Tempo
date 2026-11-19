# End-to-end tests on Wayland

Written for AI agents (and people) who need to check that Tempo really works: the real
app, on a real Wayland compositor, driven by real pointer and keyboard events, checked
through screenshots and the app's own state.

```bash
scripts/e2e.sh                      # build, make media + fixture, run every scenario
scripts/e2e.sh transitions markers  # only these (file names or parts of them)
E2E_NO_BUILD=1 scripts/e2e.sh smoke # reuse the last build
```

Exit status 0 means every scenario passed. The last lines print `n/m scenarios passed.
Report: <dir>`; the report holds everything to look at (see [Results](#results)).

## Guarantees

* **Wayland, never X11.** Each session runs `mutter --headless --wayland --no-x11` with a
  1600x1000 virtual monitor. The app gets `GDK_BACKEND=wayland`, that display's
  `WAYLAND_DISPLAY`, and no `DISPLAY`.
* **Isolated from the desktop you are sitting at.** Every session has its own temp dir
  (`/tmp/te2e-*`, removed afterwards) with its own `HOME`, `XDG_CONFIG_HOME`,
  `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME` and `XDG_RUNTIME_DIR`, its own
  D-Bus session bus (`dbus-daemon` with no service directories, so no portal or gvfs
  is started on demand), its own PipeWire with WirePlumber's `policy` profile
  (no sound cards, cameras or Bluetooth), and `GSETTINGS_BACKEND=memory`. Child processes
  get a fresh environment, never the caller's. `~/.config/tempo` is never read or written.
* **Input is not global.** Pointer and keys go through mutter's
  `org.gnome.Mutter.RemoteDesktop` on the private bus: virtual devices inside that
  headless mutter only. Nothing uses uinput, ydotool, xdotool or the real seat.
* **Screenshots are what the compositor shows**, from `org.gnome.Mutter.ScreenCast` on the
  private PipeWire (connected by file descriptor, never through the environment). Menus,
  popovers and drag icons are in them; the pointer is not drawn.
* **No sound.** `TEMPO_NO_AUDIO=1` makes the player skip its audio output.
* **Nothing is left running.** Every child gets SIGKILL if the harness dies
  (`PR_SET_PDEATHSIG`); a session stops them in order when it ends, even after a failure.
* Scenario `09_isolation` checks all of this on every run: the environment of each
  process, and that the app's and mutter's Unix sockets lead only into the session's
  temp dir (or to the read-only system bus).

## Needs

`mutter` (with `--headless`), `dbus-daemon`, `pipewire`, `wireplumber`,
`gst-launch-1.0` with the PipeWire plugin (`pipewiresrc`), `ffmpeg`, `python3` with
PyGObject (`gi`: Gio, GLib, Gst), and `ss` (iproute2). `scripts/e2e.sh` checks and names
anything missing. No root, no X server, no other compositor.

## What `scripts/e2e.sh` does

1. `cargo build --bin tempo` and the fixture writer
   (`crates/tempo-app/examples/e2e_fixture.rs`), then copies the binary to
   `$CARGO_TARGET_DIR/e2e-bin/tempo` (a shared target dir may be rebuilt by others) and
   checks it has the test channel.
2. Makes three 6 s solid-colour clips with a tone in `target/e2e-media/` (`red-6s.mp4`,
   `lime-6s.mp4`, `blue-6s.mp4`) and the fixture `target/e2e-media/fixture.tempo`:
   one 30 fps 1920x1080 timeline, tracks V1 and A1 only, the three clips back to back at
   0-4, 4-8, 8-12 s, each using the middle 4 s of its file (1 s handles, so centred
   transitions fit), picture and sound linked, no markers.
3. Runs `python3 tests/e2e/tempo_e2e.py run --project ... --report ...`. Each scenario
   gets a fresh session: compositor and app start, the fixture (copied into the session's
   HOME) opens, the scenario runs, everything is stopped.

## The scenarios

| File | Checks |
|------|--------|
| `01_smoke` | starts on the private display, fullscreen, tracks V1/A1, six clips, screen not black, clip drawn |
| `02_transitions` | drag Cross Dissolve from Effects onto the 8 s cut (centred), drag its edge (longer, still centred), click + Ctrl+C, Up (edit at 4 s), Ctrl+V (same length, centred), Backspace removes, Ctrl+Z restores, Alt+T video only, Shift+T audio only (Crossfade), Ctrl+T both |
| `03_inspector` | select a transition, type End Ratio 80, pick Ease In from the dropdown, type 20 frames, Set as Default Duration, Ctrl+T on the other cut is 20 frames |
| `04_markers` | click the ruler, Ctrl+M, type a name, type a Keyword, Enter: marker with both at the playhead; Ctrl+S writes both into the project file |
| `05_new_timeline` | Ctrl+N, Start Timecode 01:00:00:00, name, Audio Track Type Mono, Create: start timecode, mono tracks, a label reads 01:00:00:00 |
| `06_ghost_tracks` | drag a Media Pool clip into the band above V1: V2 + A2 with the clip (screenshot mid-drag shows the ghost lanes); one Ctrl+Z removes both |
| `07_trim` | nothing selected: Shift+[ trims the clip under the playhead's head, Shift+] another's tail, gaps left, Ctrl+Z twice restores |
| `08_ease_pixels` | Dip To Color dissolve from red: the viewer at 25 % reads about (126,0,0) linear and (221,0,0) with Ease In |
| `09_isolation` | the guarantees above |
| `10_transition_switch` | Inspector Video switch off: transition `enabled` false, the viewer at the cut shows a hard cut (pure red), Ctrl+Z restores |
| `11_curve_graph` | Inspector curve graph: double-click adds a key, dragging it up raises its value, right-click removes it |
| `12_marker_colours` | marker popup: 16 colour dots in Resolve order, a point marker's Duration reads 00:00:00:01, click Cocoa: marker colour Cocoa |
| `13_timeline_portrait` | Ctrl+N with Use Project Settings off, 1080 x 1920, 25 fps: timeline fps 25/1 and that resolution, the viewer picture is portrait |
| `14_edge_tooltip` | dragging a transition edge shows the "+ss:ff / ss:ff" tooltip at the pointer (mid-drag screenshot) and lengthens it |
| `15_landing_box` | dragging a Media Pool clip over V1 draws the landing box in the video clip colour (not green) on V1, green on A1 |
| `16_drop_across_audio` | a Cross Dissolve dragged across the A1 cut and onto the V1 cut is accepted and centred |

## Results

`<report>/summary.json` — `{"passed": n, "total": m, "results": [...]}`.
`<report>/<scenario>/` holds:

* `result.json`: `passed`, `error` and `traceback` on failure, `seconds`, `notes`
  (measurements the scenario recorded), `screenshots`.
* `*.png`: screenshots the scenario named, and `failure.png` of the screen at the moment
  a scenario failed. **Look at the PNGs** when judging a visual change: they are exactly
  what the compositor showed.
* `app.log` (Tempo's `tracing` output, `RUST_LOG=info`), `mutter.log`, `pipewire.log`,
  `wireplumber.log`, `dbus.log`.

The default report dir is `target/e2e-report/<date-time>` (set `E2E_REPORT` to choose).

## Writing a scenario

Add `tests/e2e/scenarios/NN_name.py` with a docstring (its first line is the scenario's
description) and `run(s)`, where `s` is a running `Session`:

```python
"""Blade: B, click a clip, it splits in two."""

from tempo_e2e import center, expect_eq


def run(s):
    s.fit_timeline()                      # Shift+Z: everything visible
    clip = s.clip("V1", 0)                # geometry of the first V1 clip
    s.key("b")                            # real key press
    x, y = center(clip["rect"])
    s.click(x, y)                         # real click at screen coordinates
    s.wait_for(lambda: len(s.track("V1")["clips"]) == 4, "the clip to split")
    s.screenshot("split")                 # saved as <report>/<scenario>/split.png
```

Rules that keep scenarios honest and stable:

* **Act like a user, check like a tester.** Every step a user does with the mouse or
  keyboard is done with `click`, `drag`, `key`, `type_text`, `fill`, `scroll`. Use
  `state()` and `geometry()` to find things and to check results. `action()` runs an
  internal step and is for setup only — never for the thing under test.
* **Find, don't hard-code.** Locate widgets by label (`widget(text=...)`,
  `row_widget("End Ratio", "GtkSpinButton")`) and timeline items by time
  (`clip`, `cut_point`, `transition_geo`). Pixel positions change with every layout tweak.
* **Wait for outcomes**, never `sleep`: `s.wait_for(lambda: ..., "what", timeout=5)`.
  Input methods already wait until the app is idle and the screen has stopped changing
  (up to 1 s), so dialog and revealer animations finish before the next step.
* Opening a panel narrows the timeline: call `fit_timeline()` after `open_panel()`.
* Wheel input over a GtkScale, GtkSpinButton or GtkDropDown changes its value.
  `row_widget` scrolls by wheeling over a plain label in the same GtkScrolledWindow.
* Steps several scenarios share live in `scenarios/_helpers.py` (`select_cut`,
  `add_transition_at_cut`, `seek_by_ruler`, `viewer_colour`); files starting with `_`
  are not scenarios.
* Times in the state are microseconds; the fixture is 30 fps (a frame is 33 334 µs).
* A different project: set `PROJECT = "/path/to.tempo"` in the scenario module.

Ad hoc, without the runner (useful to explore, or to reproduce one bug):

```bash
cd tests/e2e && python3 - <<'EOF'
import tempo_e2e as t
with t.Session(project="../../target/e2e-media/fixture.tempo", report_dir="/tmp/look") as s:
    s.key("ctrl+t"); s.screenshot("after")
    print(s.state()["project"]["timeline"]["tracks"][0]["transitions"])
EOF
```

## API reference (`tests/e2e/tempo_e2e.py`, finders in `tests/e2e/e2e_find.py`)

`Session(project=None, report_dir=None, size=(1600, 1000), app=None, extra_env=None)` —
context manager. `extra_env` adds variables for the app (e.g. `{"GDK_DEBUG": "events,dnd"}`).
After start: `s.tmp`, `s.run_dir`, `s.display`, `s.project_copy` (the file the app opened),
`s.pid(name)`.

Input (screen coordinates = window coordinates; the window is fullscreen at 0,0):

| Call | Does |
|------|------|
| `move(x, y)` | pointer motion |
| `click(x, y, button="left", count=1, mods=())` | click; `mods=("ctrl",)` holds keys during it |
| `double_click(x, y)`, `right_click(x, y)` | |
| `mouse_down(button)`, `mouse_up(button)` | |
| `drag(x1, y1, x2, y2, steps=25, duration=0.6, hold=0.2, mods=(), via=(), before_release=None)` | press, move in steps (through `via` points), wait, release; works for drag-and-drop between widgets |
| `scroll(x, y, steps=1, horizontal=False)` | wheel; positive is down/right |
| `key("ctrl+t")` | key with modifiers: `ctrl`, `shift`, `alt`, `super`; keys by character (`t`, `[`) or name (`BackSpace`, `Return`, `Escape`, `Up`, `Delete`, `Home`, `F5`, `space`, `plus`) |
| `type_text("Intro")` | types characters |
| `fill(widget, text, enter=True)` | click into a field's text, Ctrl+A, type, Return |

Looking:

| Call | Returns |
|------|---------|
| `state()` | `project` (the whole project as saved: `timeline` with `tracks` → `clips`, `transitions` with every field, `markers` with `name`/`keyword`/`color`, `start_tc_us`, `fps`; `timelines`; `sources`), `selection`, `edit_selection` (`{"kind": "edit"/"transition", ...}`), `selected_transition`, `transition_clipboard`, `playhead_us`, `playhead_tc`, `start_tc_us`, `fps`, `settings` (incl. `transition_us`), `tool`, `dirty`, `playing`, `title`, `fullscreen`, `focus` |
| `geometry()` | `window`, `fullscreen`, `widgets` (every mapped widget: `id`, `parent`, `type`, `name`, `css`, `text`, `tooltip`, `active`, `value`, `sensitive`, `focus`, `rect` [x, y, w, h]; popovers included at their screen place), `timeline` (`canvas`, `ruler`, `ghost_band`, `pps`, `playhead_x`, `tracks` [name, id, rect], `clips` [id, name, track, in_us, out_us, rect, edit_y], `transitions` [id, clip_id, kind, track, start_us, end_us, rect], `markers` [id, position_us, x]) |
| `screenshot(name=None)` | `Image` after the screen is still; `.pixel(x, y)`, `.mean(rect)`, `.brightness(rect=None)`, `.save(path)` |
| `widgets(text=, type=, tooltip=, css=, contains=)` / `widget(...)` | all matches / the first, waiting for it |
| `row_widget(label, type, index=0)` / `row_widgets(label, type)` | the field on a label's row (Inspector, dialogs), scrolled into view |
| `descendants(widget, type)`, `ancestor(widget, type, geometry)` | widget tree walking |
| `scroll_into_view(find)` | wheel the enclosing GtkScrolledWindow until `find()`'s widget shows |
| `open_panel("Effects")` | click a top-bar panel toggle unless on |
| `fit_timeline()` | Shift+Z |
| `tracks()`, `track("V1")`, `track_name(t)`, `transitions("V1")`, `timeline()` | from `state()` |
| `clip("V1", i)`, `transition_geo("V1", i)`, `cut_point("V1", i)` | from `geometry()`, left to right |
| `wait_for(fn, what, timeout=5)`, `wait_idle()`, `settle(quiet=0.25, timeout=5)` | waiting; `settle` returns when two frames `quiet` apart have the same pixels |
| `action(step)` | setup only: a GAction name (`win.zoom-fit`) or a `TEMPO_ACTIONS` step (`seek:2.5`) |
| `note(key, value)` | put a measurement in `result.json` |

Assertions: `expect(cond, msg)`, `expect_eq(actual, expected, what)`,
`expect_near(actual, expected, tolerance, what)`; they raise `E2EError`.

## The app side

All in `crates/tempo-ui/src/app.rs`, active only when the variable is set:

* `TEMPO_TEST_CONTROL=/dir/control.sock` — a Unix socket served with GIO on the GTK main
  loop. One command per line, one JSON reply per line (`{"ok": true, "result": ...}` or
  `{"ok": false, "error": "..."}`): `ping`, `state`, `geometry`, `action <step>`,
  `wait-idle`, `quit`. Try it by hand: `printf 'state\n' | nc -U /tmp/te2e-*/run/control.sock`.
* `TEMPO_TEST_FULLSCREEN=1` — the window starts fullscreen, so window = screen coordinates.
* `TEMPO_NO_AUDIO=1` (in `player.rs`) — no audio output is opened.

## Troubleshooting

| Symptom | Cause and fix |
|---------|---------------|
| `timed out ... the app's control socket` | The binary has no test channel (another checkout rebuilt `target/debug/tempo`): run `scripts/e2e.sh` without `E2E_NO_BUILD`. Read `app.log`. |
| `socket path ... exceeds 108 bytes` in `mutter.log` | Session dirs live in `/tmp`; set `TEMPO_E2E_TMP` to another short dir if `/tmp` is unusable. |
| `X exited with N` | That process died; its log tail is in the message and in the report dir. |
| A click or key does nothing | Is the point on screen (`geometry()`), the panel open, the timeline fitted? `extra_env={"GDK_DEBUG": "events"}` logs every event the app gets to `app.log`. |
| A drop does nothing | The target refused it: a Cross Dissolve cannot go where there is no media to overlap (no handles) or over an audio track. `GDK_DEBUG=dnd` shows accept/refuse in `app.log`; `drag(..., via=[...])` steers around refusing places. |
| Screenshot older than expected | Screenshots wait for the app to be idle and the pixels to be unchanged for 0.25 s; for slow work (decoding) `wait_for` a state change first. |

## Known limits

* One monitor, scale 1, no Xwayland (X11-only code paths are not exercised).
* The pointer is not drawn in screenshots (hover effects still are).
* No `xdg-desktop-portal`: file choosers and similar portal dialogs cannot be tested.
* No sound is played or checked; audio code runs only as far as the timeline model.
* Pixel checks assume Tempo's fixed dark theme and the solid-colour fixture.
* Each scenario starts its own compositor and app (about 3 s); keep scenarios focused.
