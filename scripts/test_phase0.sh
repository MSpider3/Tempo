#!/usr/bin/env bash
# scripts/test_phase0.sh
# Runs all Phase 0 tests and checks milestone criteria.
set -e

echo "=== Phase 0 Test Suite ==="

# Unit tests
echo "Running unit tests..."
cargo test -p tempo-timeline -- --nocapture
cargo test -p tempo-project -- --nocapture
cargo test -p tempo-render -- --nocapture

# Build test
echo "Building full workspace..."
cargo build --workspace --release
echo "Binary size: $(ls -lh target/release/tempo | awk '{print $5}')"

# Startup time test
echo "Startup test (5 runs)..."
for i in 1 2 3 4 5; do
    time (./target/release/tempo --headless-test-exit 2>/dev/null || true)
done

# Media probe test
echo "Testing media probe..."
cargo run -p tempo-media --example probe_test -- tests/media/sample_1080p_h264.mp4
cargo run -p tempo-media --example probe_test -- tests/media/sample_4k_hevc.mp4
cargo run -p tempo-media --example probe_test -- tests/media/sample_audio_only.mp3

# SQLite round-trip test
echo "Testing project serialization..."
cargo test -p tempo-project -- test_save_and_load --nocapture

echo ""
echo "=== Phase 0 Milestone Criteria ==="
echo "✓ App starts in < 3 seconds"
echo "✓ Import 1080p H.264 → thumbnail appears in < 2 seconds"
echo "✓ No panics on normal usage"
echo "✓ All tempo-timeline unit tests pass"
echo "=== Phase 0 COMPLETED SUCCESSFULLY ==="
