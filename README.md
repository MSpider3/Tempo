# Tempo

**Lightweight, CPU-only video editor inspired by DaVinci Resolve's Cut Page.**

Tempo is a beginner-friendly desktop video editor that clones the workflow and
keybindings of DaVinci Resolve's Cut Page — stripped down to only the essentials.
No GPU required. Runs on any modern laptop.

---

## ✨ Features

- 🎬 **Dual Timeline** — Overview + detail timeline, exactly like Resolve's Cut Page
- 🎚️ **Multi-Track Editing** — 3 video tracks, 3 audio tracks, 1 text track
- ✂️ **Blade, Trim, Ripple** — Professional editing tools with DaVinci keybindings
- ⚡ **Proxy-Based Editing** — Auto-generates low-res proxies for smooth playback
- 🖥️ **CPU-Only** — No GPU needed. Runs on any modern laptop
- 🎨 **Dark Professional Theme** — Clean, DaVinci-inspired dark UI
- 🔤 **Text/Title Overlays** — Full text editor with font, color, position, rotation
- 🎞️ **7 Transitions** — Fade In, Fade Out, Cross Dissolve, Cut to Black/White, Crossfade
- ⏩ **Speed Control** — 0.25× to 4× with pitch correction
- 📦 **Export to MP4** — H.264 + AAC, multiple resolutions (480p–Original)
- ↩️ **Unlimited Undo/Redo** — Full edit history with Command pattern
- ⌨️ **DaVinci Resolve Keybindings** — Your muscle memory transfers directly

---

## 📋 Requirements

| Dependency | Version | Purpose |
|-----------|---------|---------|
| **Python** | 3.11+ | Runtime |
| **FFmpeg** | 5.0+ | Video processing, proxy generation, export |
| **MPV** | 0.36+ | Video preview playback |
| **uv** | Latest | Python package manager |

### Install System Dependencies

**Arch Linux / Manjaro:**
```bash
sudo pacman -S ffmpeg mpv
```

**Ubuntu / Debian:**
```bash
sudo apt install ffmpeg mpv libmpv-dev
```

**Fedora:**
```bash
sudo dnf install ffmpeg mpv mpv-libs-devel
```

**macOS:**
```bash
brew install ffmpeg mpv
```

**Windows:**
Download FFmpeg from [ffmpeg.org](https://ffmpeg.org/download.html) and MPV from
[mpv.io](https://mpv.io/installation/). Add both to your PATH.

---

## 🚀 Installation

```bash
# Clone the repository
git clone https://github.com/mehulgolecha/tempo.git
cd tempo

# Install dependencies with uv
uv sync

# (Optional) Install API dependencies for external tool integration
uv sync --extra api
```

---

## ▶️ Running

```bash
# Run with uv
uv run tempo

# Or run as a Python module
python -m tempo

# Or use the installed script
tempo
```

---

## 🛠️ Development

### Setup

```bash
# Install with dev dependencies
uv sync --dev

# Run the application
uv run tempo
```

### Linting

```bash
# Run ruff linter
uv run ruff check .

# Auto-fix issues
uv run ruff check --fix .

# Format code
uv run ruff format .
```

### Type Checking

```bash
# Run mypy in strict mode
uv run mypy tempo/
```

### Testing

```bash
# Run all tests
uv run pytest

# Run with verbose output
uv run pytest -v

# Run specific test file
uv run pytest tests/test_models.py
```

---

## ⌨️ Keybindings

Tempo uses **DaVinci Resolve Cut Page keybindings**. Your muscle memory transfers
directly.

| Key | Action |
|-----|--------|
| `Space` | Play / Pause |
| `J` / `K` / `L` | Rewind / Stop / Fast Forward (shuttle) |
| `I` / `O` | Mark In / Out point |
| `B` | Blade tool (split at playhead) |
| `A` | Select tool |
| `T` | Trim tool |
| `Ctrl+Z` | Undo |
| `Ctrl+Shift+Z` | Redo |
| `Ctrl+C/X/V` | Copy / Cut / Paste |
| `Delete` | Ripple delete |
| `Backspace` | Delete (leave gap) |
| `Ctrl+I` | Import media |
| `Ctrl+E` | Export |
| `Ctrl+S` | Save project |

📖 **Full keybinding reference:** [KEYBINDINGS.md](KEYBINDINGS.md)

---

## 📁 Project Structure

```
tempo/
├── SPEC.md                    # Product specification
├── ARCHITECTURE.md            # Technical architecture
├── ROADMAP.md                 # Development roadmap
├── KEYBINDINGS.md             # Keybinding reference
├── pyproject.toml             # Project config (uv)
│
├── tempo/                     # Main Python package
│   ├── __main__.py            # Entry point
│   ├── app.py                 # QApplication setup
│   ├── core/                  # Business logic (Qt-free)
│   │   ├── models.py          # Clip, Track, Project dataclasses
│   │   ├── project.py         # Save/load JSON project files
│   │   ├── timeline.py        # Timeline operations
│   │   ├── text_overlay.py    # Text clip logic
│   │   ├── speed.py           # Speed multiplier logic
│   │   └── edit_history.py    # Undo/redo (Command pattern)
│   ├── media/                 # Media handling
│   │   ├── importer.py        # File import pipeline
│   │   ├── proxy.py           # Proxy generation (background)
│   │   ├── thumbnail.py       # Thumbnail extraction
│   │   ├── waveform.py        # Waveform data extraction
│   │   └── exporter.py        # FFmpeg export engine
│   ├── ui/                    # PySide6 UI
│   │   ├── main_window.py     # Main window layout
│   │   ├── theme.py           # Dark theme + color system
│   │   ├── keybindings.py     # Centralized shortcut registration
│   │   ├── panels/            # Media bin, preview, inspector
│   │   └── timeline/          # Timeline widgets
│   └── utils/                 # Utilities
│       ├── ffmpeg.py          # FFmpeg subprocess wrapper
│       ├── timecode.py        # Time formatting helpers
│       ├── cache.py           # Proxy/thumbnail cache
│       └── logger.py          # Logging setup
│
├── assets/                    # Icons and fonts
├── tests/                     # Test suite
└── scripts/                   # Build and dev scripts
```

---

## 🏗️ Tech Stack

| Component | Technology |
|-----------|-----------|
| Language | Python 3.11+ |
| UI Framework | PySide6 (Qt6) |
| Video Engine | FFmpeg (subprocess) |
| Preview Player | MPV (python-mpv) |
| Data Model | Python dataclasses + JSON |
| Package Manager | uv |
| Linter | ruff |
| Type Checker | mypy (strict) |
| Testing | pytest + pytest-qt |
| Packaging | PyInstaller |
| Hot Paths (Phase 4) | Rust via PyO3 |

---

## 📄 Project Files

Tempo projects are saved as `.tempo` files (JSON format). See the
[specification](SPEC.md) for the complete file format.

---

## 🤝 Contributing

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Ensure code passes linting and type checks:
   ```bash
   uv run ruff check .
   uv run mypy tempo/
   uv run pytest
   ```
4. Commit your changes (`git commit -m 'Add amazing feature'`)
5. Push to the branch (`git push origin feature/amazing-feature`)
6. Open a Pull Request

### Code Standards

- **Type hints everywhere** — `mypy --strict` must pass
- **No Qt imports in `core/`** — Business logic is framework-independent
- **All FFmpeg calls through `utils/ffmpeg.py`** — No direct subprocess calls
- **Colors and fonts from `ui/theme.py`** — No hardcoded hex values in widgets
- **Keybindings in `ui/keybindings.py`** — Single source for all shortcuts

---

## 📝 License

This project is licensed under the MIT License — see the [LICENSE](LICENSE) file
for details.

---

## 📚 Documentation

- [Product Specification](SPEC.md)
- [Technical Architecture](ARCHITECTURE.md)
- [Development Roadmap](ROADMAP.md)
- [Keybinding Reference](KEYBINDINGS.md)
