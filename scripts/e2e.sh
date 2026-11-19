#!/usr/bin/env bash
# End-to-end tests of the real app on a private, headless Wayland compositor.
# See docs/TESTING_E2E.md.   Usage: scripts/e2e.sh [scenario ...]   (default: all)
#   E2E_REPORT=dir   where results go (default target/e2e-report/<date-time>)
#   E2E_NO_BUILD=1   use the binaries already built
set -euo pipefail

cd "$(dirname "$0")/.."
repo=$PWD
target=${CARGO_TARGET_DIR:-$repo/target}
media=$target/e2e-media
report=${E2E_REPORT:-$target/e2e-report/$(date +%Y%m%d-%H%M%S)}

for tool in mutter dbus-daemon pipewire wireplumber gst-launch-1.0 ffmpeg python3; do
    command -v "$tool" >/dev/null || { echo "e2e: missing $tool (see docs/TESTING_E2E.md)" >&2; exit 2; }
done
gst-inspect-1.0 pipewiresrc >/dev/null 2>&1 || { echo "e2e: missing the GStreamer PipeWire plugin (pipewiresrc)" >&2; exit 2; }
python3 -c 'import gi; gi.require_version("Gst", "1.0"); from gi.repository import Gst, Gio' 2>/dev/null ||
    { echo "e2e: python3 needs PyGObject with Gst 1.0" >&2; exit 2; }

# The target dir may be shared with other checkouts, so the tested binary is copied
# aside right after the build and checked for the test channel.
app=$target/e2e-bin/tempo
if [ -z "${E2E_NO_BUILD:-}" ]; then
    cargo build --bin tempo
    cargo build -p tempo-app --example e2e_fixture
    mkdir -p "$target/e2e-bin"
    cp "$target/debug/tempo" "$app"
fi
grep -aq TEMPO_TEST_CONTROL "$app" 2>/dev/null || { echo "e2e: $app has no test control channel; build again" >&2; exit 2; }

# Plain colour clips with a tone, so pixel checks are exact: 6 s at 30 fps, of which the
# fixture uses the middle 4 s (the rest are handles for transitions).
mkdir -p "$media"
if ffmpeg -hide_banner -encoders 2>/dev/null | grep -q libx264; then h264=libx264; else h264=libopenh264; fi
for colour in red lime blue; do
    [ -f "$media/$colour-6s.mp4" ] && continue
    ffmpeg -v error -y -f lavfi -i "color=c=$colour:size=640x360:rate=30:duration=6" \
        -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=6" \
        -pix_fmt yuv420p -g 30 -c:v "$h264" -c:a aac -shortest "$media/$colour-6s.mp4"
done
project=$media/fixture.tempo
rm -f "$project"
"$target/debug/examples/e2e_fixture" "$project" "$media/red-6s.mp4" "$media/lime-6s.mp4" "$media/blue-6s.mp4"

mkdir -p "$report"
python3 tests/e2e/tempo_e2e.py run --project "$project" --report "$report" --app "$app" "$@"
