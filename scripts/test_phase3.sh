#!/usr/bin/env bash
# scripts/test_phase3.sh
set -e

echo "=== Phase 3 Test Suite ==="

cargo test --workspace -- --nocapture

echo ""
echo "=== Multi-Track Compositing Test ==="
cargo run -p tempo-render --example multitrack_composite_test -- \
    tests/media/sample_1080p_h264.mp4 \
    --tracks 4 \
    --frames 100
# Renders 100 frames of a 4-track timeline and checks:
# - No dropped frames
# - Frame composition time < 16ms per frame
# - Memory usage < 2 GB

echo ""
echo "=== Keyframe Interpolation Test ==="
cargo test -p tempo-timeline -- keyframe_interpolation --nocapture
# Tests: linear interpolation at various positions, boundary conditions

echo ""
echo "=== Title Render Test ==="
cargo run -p tempo-render --example title_render_test
# Renders Center Title and Lower Third → saves as PNG → verify text is present
# (basic check: pixel count of non-black pixels > threshold)

echo ""
echo "=== Page Switch State Test (3-Way) ==="
cargo test -p tempo-ui -- --nocapture   # keybind table, timecode
# Test: apply edits in Cut Page, switch to Edit Page, switch to Export Page, verify timeline & playback state preserved

echo ""
echo "=== Inspector Round-Trip Test ==="
cargo test -p tempo-project -- inspector_round_trip --nocapture
# Set opacity=0.5, position=(100,-50), scale=(1.5,1.5), rotation=15.0
# Save project → reload → verify all values exactly match

echo ""
echo "=== Color Correction Render Test ==="
cargo run -p tempo-render --example color_correction_test -- \
    tests/media/sample_1080p_h264.mp4
# Applies lift/gamma/gain to a known frame, compares with expected output.
# Expected outputs are pre-computed reference PNGs in tests/render_references/

echo ""
echo "=== RenderQueue Lifecycle & State Machine Test ==="
cargo test -p tempo-export -- render_queue_lifecycle --nocapture
# Tests: job states Queued -> Rendering -> Paused -> Completed / Failed / Cancelled

echo ""
echo "=== Export Cancellation Latency & Scratch Cleanup Test ==="
cargo test -p tempo-export -- export_cancellation_latency --nocapture
# Tests: job cancellation takes <= 42ms and cleanly deletes partial files

echo ""
echo "=== Background Export Continuity Test ==="
cargo test -p tempo-export -- timeline_export --nocapture
# Tests: timeline scrubbing continues with zero frame drops during active export

echo ""
echo "=== Phase 3 Milestone Criteria (manual) ==="
echo "1. 4 video + 3 audio track timeline plays at 30fps on target hardware"
echo "2. Opacity keyframe (fade in over 2s): visible in export"
echo "3. Center Title + Lower Third: text renders correctly in export"
echo "4. Switch Cut ↔ Edit ↔ Export 10 times: timeline and playhead identical each time"
echo "5. All clip properties survive save/reload"
echo "6. Memory ceiling: < 2 GB for 4-video-track timeline with titles"
echo "7. Export Workspace Batch Test: Queue 3 presets (YouTube, ProRes, Custom), render sequentially"
echo "8. Background Continuity: Active 4K export does not interrupt editing/scrubbing on Cut/Edit pages"
echo "9. Cancellation Latency: Cancel in-flight job cleanly within 42ms"

