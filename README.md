# Tempo

**A light video editor for Linux that works like DaVinci Resolve's Edit page.**

Tempo is for people who want to start editing video on an ordinary laptop. It uses the
same layout and the same keyboard shortcuts as DaVinci Resolve, so everything you learn in
Tempo carries over when you move to Resolve later. It also works well as a small, quick
editor for cutting a video and posting it.

- **Runs on weak hardware.** No graphics card needed. Developed and measured on a
  dual-core Intel Core i3 with integrated graphics.
- **No waiting on import.** Files are usable at once; nothing is converted first.
- **Same keys as Resolve.** Every shared shortcut is checked against a key file exported
  from DaVinci Resolve 20.

> **Alpha release.** Tempo is version 0.2 and is not stable yet. Expect bugs, rough
> edges and changes between versions, and keep a copy of anything you cannot afford to
> lose. Do not rely on it for important work. The downloads on the Releases page are
> marked as pre-releases for this reason. See
> [what is not done yet](#what-is-not-done-yet), and please
> [report what goes wrong](https://github.com/MSpider3/Tempo/issues).

## What it does

- **Project Manager** to create and open projects.
- **Edit page** with a Media Pool, viewer, Inspector and timeline, arranged as in Resolve.
- **Editing:** insert, overwrite, append, place on top, move, trim, blade, razor, ripple
  delete, snapping, nudge. Every edit can be undone.
- **Change Clip Speed** (`R`, as in Resolve): 25 % to 400 %, with Ripple Sequence to let
  later clips move. Sound follows the speed.
- **Fade handles:** hover a clip and drag a top corner toward the middle.
- **Track controls:** Lock on every track, show/hide on video tracks, and Mute and Solo
  (for listening) on audio tracks.
- **Preview with sound**, with the picture following the audio clock.
- **Right-click menus** on timeline clips, on gaps and in the Media Pool, for the edits a
  beginner looks for first.
- **Linked picture and sound:** a clip and its sound move, cut, trim and delete together
  until you unlink them.
- **Proxies and preview quality:**
  - Heavy footage on the timeline gets a small 540p copy with sound, made in the background.
    It is made on the graphics chip when the machine supports it (VA-API).
  - The **Preview** button above the viewer switches between Full (the original file), Half
    and Quarter; Half is the default.
  - When playback can't keep up, it decodes more cheaply until you stop.
  - Export always reads the original files.
- **Matches the project to your footage:** the first video brought into an empty project
  offers to set the project's frame rate and size to match, as Resolve does.
- **Project backups:** a checked copy every 10 minutes while you edit, kept like Resolve's
  (recent, hourly, daily). Right-click a project and choose **Project Backups…** to open one.
- **Shortcut hints** in the bottom bar for whatever is under the pointer.
- **Markers that become chapters:** name a marker and it is written into the exported file
  and offered as a ready-to-paste timestamp list, with YouTube's rules checked.
- **Export page:** pick an output by frame shape — 16:9, 9:16, 1:1 or 4:5 — or **≤ 10 MB**
  for Discord and email, queue several, and keep editing while they render.

## Install

Packages for each release are on the
[Releases page](https://github.com/MSpider3/Tempo/releases): an AppImage, a Flatpak bundle,
an `.rpm` and a `.deb`, with a `SHA256SUMS` file to verify the download:

```bash
sha256sum --check --ignore-missing SHA256SUMS
```

Tempo needs the `ffmpeg` program at run time (the `.deb` and `.rpm` depend on it; the
AppImage carries its own).

## Build from source

You need Rust, GTK 4.14 or newer, libadwaita 1.6 or newer, PipeWire and FFmpeg.

**Fedora**

```bash
sudo dnf install gcc clang-devel rust cargo gtk4-devel libadwaita-devel glib2-devel \
    pipewire-devel ffmpeg-free ffmpeg-free-devel
```

**Ubuntu 25.04 or newer**

```bash
sudo apt install build-essential clang libclang-dev pkg-config libgtk-4-dev \
    libadwaita-1-dev libglib2.0-dev-bin libpipewire-0.3-dev libavcodec-dev \
    libavformat-dev libavutil-dev libswscale-dev libswresample-dev libavfilter-dev \
    libavdevice-dev ffmpeg
```

Then:

```bash
./scripts/check_env.sh        # confirms everything is installed
cargo run --release --bin tempo
```

`./scripts/run_all_tests.sh` builds everything and runs every test.

## Keyboard shortcuts

The same as DaVinci Resolve. A few to start with:

| Key | Action |
|---|---|
| `Space` | Play / stop |
| `J` `K` `L` | Play reverse, stop, play forward (press again to go faster) |
| `A` `T` `B` | Selection, Trim, Blade |
| `R` | Change clip speed |
| `Ctrl+B` | Cut at the playhead |
| `Backspace` | Delete, leaving a gap |
| `Shift+Backspace` | Delete and close the gap |
| `F9` `F10` | Insert, Overwrite |
| `I` `O` | Mark In, Mark Out |
| `M` | Add a marker |
| `N` | Snapping on / off |
| `Shift+4` `Shift+8` | Edit page, Export page |
| `F1` | Show all shortcuts |

The full list is in [docs/KEYBINDS.md](docs/KEYBINDS.md).

## What is not done yet

- **Hardware (VA-API) decoding is off by default.** On the test laptop it halves CPU use,
  but drops more frames than the processor on 4K footage (see [docs/PERF.md](docs/PERF.md)).
  Proxies are made on the graphics chip when a start-up check finds it works. Hardware
  export encoding is a switch in Preferences, off by default.
- Change Clip Speed has no Reverse, Freeze Frame or Pitch Correction yet.
- Transitions other than Cross Dissolve, and title styles, are planned as plugins; those
  plugin types are not built yet.
- No upload plugin for a real site ships yet. The plugin point works (see
  [docs/PLUGIN_SPEC.md](docs/PLUGIN_SPEC.md)), but each site needs its own developer keys.
- Keyframe animation.
- The Inspector changes one clip at a time, also when several are selected.
- Some DaVinci Resolve keys are listed in [docs/KEYBINDS.md](docs/KEYBINDS.md) but not
  built yet (loop playback, paste insert, select forward, and others); that file marks them.

The complete, current list is at the top of [docs/ROADMAP.md](docs/ROADMAP.md).

## How it is built

| | |
|---|---|
| Language | Rust |
| Interface | GTK 4 and libadwaita |
| Decoding | FFmpeg libraries |
| Export and proxies | the `ffmpeg` program |
| Sound | PipeWire |
| Project file | SQLite, one `.tempo` file |

```
crates/
  tempo-timeline/   data model and undoable commands
  tempo-media/      probing, video and audio decoding
  tempo-audio/      PipeWire output
  tempo-proxy/      background proxy files
  tempo-export/     export, chapters
  tempo-project/    project file
  tempo-ui/         the interface
  tempo-app/        the program
```

Design and behaviour are described in [docs/](docs/README.md).

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) first. Tempo is for weaker computers and for
beginners. It keeps DaVinci Resolve's layout and keys on purpose, and effects and titles
come as plugins. The guide explains what that means for a change, and lists the checks to
run before opening a pull request.

## Licence

Tempo is free software under the GNU General Public License, version 3. See [LICENSE](LICENSE).

DaVinci Resolve is a trademark of Blackmagic Design. Tempo is an independent project and is
not affiliated with or endorsed by Blackmagic Design.
