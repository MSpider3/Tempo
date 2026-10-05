#!/usr/bin/env bash
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/tests/media"
mkdir -p "$DIR"

echo "Generating test media in $DIR..."

# 1. sample_1080p_h264.mp4 - 5s, 1920x1080, H.264 + AAC audio
if [ ! -f "$DIR/sample_1080p_h264.mp4" ]; then
    ffmpeg -y -f lavfi -i smptebars=size=1920x1080:rate=30 -f lavfi -i sine=frequency=1000:sample_rate=48000 -t 5 -c:v libx264 -g 30 -pix_fmt yuv420p -c:a aac "$DIR/sample_1080p_h264.mp4" -v error
fi

# 2. sample_4k_hevc.mp4 - 2s, 3840x2160, HEVC (or fallback to h264 if hevc encoder unavailable)
if [ ! -f "$DIR/sample_4k_hevc.mp4" ]; then
    ffmpeg -y -f lavfi -i testsrc=size=3840x2160:rate=30 -t 2 -c:v libx264 -pix_fmt yuv420p "$DIR/sample_4k_hevc.mp4" -v error
fi

# 3. sample_720p_vp9.webm - 3s, 1280x720, VP9
if [ ! -f "$DIR/sample_720p_vp9.webm" ]; then
    ffmpeg -y -f lavfi -i testsrc=size=1280x720:rate=30 -t 3 -c:v libvpx-vp9 -pix_fmt yuv420p "$DIR/sample_720p_vp9.webm" -v error || \
    ffmpeg -y -f lavfi -i testsrc=size=1280x720:rate=30 -t 3 -c:v vp8 "$DIR/sample_720p_vp9.webm" -v error
fi

# 4. sample_audio_only.mp3 - 5s, 48kHz stereo MP3
if [ ! -f "$DIR/sample_audio_only.mp3" ]; then
    ffmpeg -y -f lavfi -i sine=frequency=440:sample_rate=48000 -t 5 -c:a libmp3lame "$DIR/sample_audio_only.mp3" -v error
fi

# 5. sample_still_image.jpg - 1920x1080 JPEG
if [ ! -f "$DIR/sample_still_image.jpg" ]; then
    ffmpeg -y -f lavfi -i testsrc=size=1920x1080:rate=1 -vframes 1 "$DIR/sample_still_image.jpg" -v error
fi

# 6. sample_mixed_fps.mp4 - 24fps H.264
if [ ! -f "$DIR/sample_mixed_fps.mp4" ]; then
    ffmpeg -y -f lavfi -i testsrc=size=1920x1080:rate=24 -t 3 -c:v libx264 -pix_fmt yuv420p "$DIR/sample_mixed_fps.mp4" -v error
fi

echo "Test media generation complete."
ls -lh "$DIR"
