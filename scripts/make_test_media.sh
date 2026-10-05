#!/usr/bin/env bash
# Makes the small clips the media tests read, in target/test-media/.
# The tests skip themselves when the clips are missing, so run this first.
set -euo pipefail

cd "$(dirname "$0")/.."
out=target/test-media
mkdir -p "$out"

# Fedora's FFmpeg has OpenH264 instead of x264.
if ffmpeg -hide_banner -encoders 2>/dev/null | grep -q libx264; then
    h264=libx264
else
    h264=libopenh264
fi

make() { # name, seconds, size, then extra output options
    local name=$1 seconds=$2 size=$3
    shift 3
    [ -f "$out/$name" ] && return 0
    ffmpeg -v error -y \
        -f lavfi -i "testsrc2=size=$size:rate=30:duration=$seconds" \
        -f lavfi -i "sine=frequency=440:beep_factor=4:sample_rate=48000:duration=$seconds" \
        -pix_fmt yuv420p -g 30 -ac 1 -shortest "$@" "$out/$name"
}

make sample_1080p_h264.mp4 5 1920x1080 -c:v "$h264" -b:v 1M -c:a aac
make sample_10s_sync.mp4 12 1920x1080 -c:v "$h264" -b:v 1M -c:a aac
# Only the demo project uses this one, so a build without VP9 is not an error.
make sample_720p_vp9.webm 3 1280x720 -c:v libvpx-vp9 -b:v 500k -c:a libopus || echo "skipped the VP9 sample"

ls -l "$out"
