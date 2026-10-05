#!/usr/bin/env bash
# scripts/test_phase2.sh
# Runs all Phase 2 tests and checks milestone criteria.
set -e

echo "========================================================"
echo "=== Phase 2 Automated Verification Suite (Cut Page) ==="
echo "========================================================"

echo ""
echo "1/12. Testing Timeline Editing Operations..."
cargo test -p tempo-timeline -- editing_operations --nocapture

echo ""
echo "2/12. Testing Timeline Undo/Redo Stress..."
cargo test -p tempo-timeline -- undo_redo_stress --nocapture

echo ""
echo "3/12. Testing Multi-Track Compositor..."
cargo test -p tempo-render -- multi_track_compositor --nocapture

echo ""
echo "4/12. Testing Transition Shaders..."
cargo test -p tempo-render -- transition_shader --nocapture

echo ""
echo "5/12. Testing Audio Peak & RMS Meters..."
cargo test -p tempo-audio -- audio_meters --nocapture

echo ""
echo "6/12. Testing Waveform Generation & Mipmaps..."
cargo test -p tempo-audio -- waveform_generation --nocapture

echo ""
echo "7/12. Testing SQLite Full Project Round-Trip..."
cargo test -p tempo-project -- full_project_round_trip --nocapture

echo ""
echo "8/12. Testing Atomic Auto-Save & Recovery..."
cargo test -p tempo-project -- autosave_recovery --nocapture

echo ""
echo "9/12. Testing Proxy Generation Engine..."
cargo test -p tempo-media -- proxy_generation --nocapture

echo ""
echo "10/12. Testing H.264 Video Export Engine..."
cargo test -p tempo-export -- export_h264 --nocapture

echo ""
echo "11/12. Testing CMX 3600 EDL Export Engine..."
cargo test -p tempo-export -- edl_export --nocapture

echo ""
echo "12/12. Running UI Smoke Test (Launch tempo, verify Cut page renders, exit 0)..."
cargo run -p tempo-app -- --smoke-test

echo ""
echo "========================================================"
echo "=== Phase 2 Milestone Criteria Verification Summary ==="
echo "========================================================"
echo "✓ 1. Timeline Editing Operations (insert, overwrite, append, replace, ripple delete, lift) passed"
echo "✓ 2. Undo / Redo Stress test (100 sequential operations + reversibility) passed"
echo "✓ 3. Multi-Track Compositor (V2 over V1 with alpha blending & transforms) passed"
echo "✓ 4. Transition Shader (Cross dissolve, dip to black/white, fade in/out) passed"
echo "✓ 5. Audio Meters (Peak dBFS, RMS dBFS, clipping indicator) passed"
echo "✓ 6. Waveform Generation & Mipmaps (1x, 4x, 16x, 64x mip pyramids in <50ms) passed"
echo "✓ 7. SQLite Full Project Round-Trip (ACID persistence) passed"
echo "✓ 8. Atomic Auto-Save & Crash Recovery passed"
echo "✓ 9. Proxy Generation Engine (H.264 proxy with codec verification) passed"
echo "✓ 10. H.264 Video Export Engine (1080p MP4 render pipeline) passed"
echo "✓ 11. CMX 3600 EDL Export Engine (SMPTE timecode formatting) passed"
echo "✓ 12. UI Smoke Test (GTK4 + libadwaita Cut page launch, toolbar, keybinds, exit 0) passed"
echo "=== PHASE 2 COMPLETED SUCCESSFULLY ==="
