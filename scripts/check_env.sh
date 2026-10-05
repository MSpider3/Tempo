#!/usr/bin/env bash
# scripts/check_env.sh
set -e
echo "=== Tempo 2 Environment Check ==="

# Rust toolchain
rustc --version | grep -E "1\.(8[2-9]|9[0-9])" || { echo "ERROR: Rust >= 1.82 required"; exit 1; }
cargo --version

# Required system libraries (Ubuntu/Fedora/Arch)
pkg-config --exists gtk4 || { echo "ERROR: GTK4 dev headers missing. Install: libgtk-4-dev (Ubuntu) or gtk4-devel (Fedora)"; exit 1; }
pkg-config --exists libadwaita-1 || { echo "ERROR: libadwaita dev headers missing. Install: libadwaita-1-dev / libadwaita-devel"; exit 1; }
pkg-config --exists libavcodec libavformat libavutil libswscale || { echo "ERROR: FFmpeg dev headers missing. Install: libavcodec-dev libavformat-dev libavutil-dev libswscale-dev / ffmpeg-devel"; exit 1; }
pkg-config --exists libpipewire-0.3 || { echo "ERROR: PipeWire dev headers missing. Install: libpipewire-0.3-dev / pipewire-devel"; exit 1; }

# Vulkan (required for wgpu primary backend)
vulkaninfo > /dev/null 2>&1 && echo "Vulkan: available" || echo "WARNING: Vulkan not available — wgpu will fall back to OpenGL. This is acceptable."

# VAAPI check
ls /dev/dri/renderD* > /dev/null 2>&1 && echo "VAAPI: render nodes found: $(ls /dev/dri/renderD*)" || echo "WARNING: No DRI render nodes — VAAPI will be unavailable (software decode only)"

# Optional: Python for compute server
python3 --version > /dev/null 2>&1 && echo "Python: available" || echo "INFO: Python not found — compute plugins will be unavailable (not required for core)"

echo ""
echo "=== Environment check complete ==="
