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

> Tempo is early software (version 0.1). The basics work; see
> [what is not done yet](#what-is-not-done-yet) before relying on it.

## What it does

- **Project Manager** to create and open projects.
- **Edit page** with a Media Pool, viewer, Inspector and timeline, arranged as in Resolve.
- **Editing:** insert, overwrite, append, place on top, move, trim, blade, razor, ripple
  delete, snapping, nudge. Every edit can be undone.
- **Preview with sound**, with the picture following the audio clock.
- **Proxies:** heavy footage gets a small 540p copy made in the background, used for preview
  only. Export always reads the original files.
- **Markers that become chapters:** name a marker and it is written into the exported file
  and offered as a ready-to-paste timestamp list, with YouTube's rules checked.
- **Export page:** pick an output by frame shape — 16:9, 9:16, 1:1 or 4:5 — queue several,
  and keep editing while they render.

## Install

Packages for each release are on the
[Releases page](https://github.com/MSpider3/Tempo/releases): an AppImage, an `.rpm` and a
`.deb`, with a `SHA256SUMS` file to verify the download:

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

- Hardware (VA-API) decoding and encoding exist but are switched off by default: they
  have not yet been tried on a machine with a working driver.
- Plugins can add timeline commands; they cannot yet add effects or upload targets.
- Keyframe animation.

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

Read [docs/AGENT_PROMPT.md](docs/AGENT_PROMPT.md) first; it lists the rules the code
follows (nothing slow on the interface thread, every edit undoable, Resolve's keys).
Run `./scripts/run_all_tests.sh` before opening a pull request.

## Licence

Tempo is free software under the GNU General Public License, version 3. See [LICENSE](LICENSE).

DaVinci Resolve is a trademark of Blackmagic Design. Tempo is an independent project and is
not affiliated with or endorsed by Blackmagic Design.
