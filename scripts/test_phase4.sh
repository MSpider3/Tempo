#!/usr/bin/env bash
# ==============================================================================
# Tempo Video Editor - Phase 4 Automated Test Suite
# Tests: WASM Plugin System, Lua Code Mode, Python Compute Server, & Effect Pipeline
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

PASSED_TESTS=0
TOTAL_TESTS=6

echo -e "${BLUE}=== Tempo Phase 4 Test Suite ===${NC}\n"

# 1. Manifest Parsing Test
echo -e "${BLUE}[1/6] Testing Plugin Manifest TOML Parser...${NC}"
cargo test -p tempo-plugin --lib -- manifest --nocapture
echo -e "${GREEN}✓ Manifest parsing verified${NC}\n"
PASSED_TESTS=$((PASSED_TESTS + 1))

# 2. Plugin Crash Isolation Test
echo -e "${BLUE}[2/6] Testing Plugin Crash Isolation (Trap Protection)...${NC}"
cargo test -p tempo-plugin --test plugin_tests -- plugin_crash_isolation --nocapture
echo -e "${GREEN}✓ Plugin crash isolation verified (WASM traps caught cleanly, host didn't crash)${NC}\n"
PASSED_TESTS=$((PASSED_TESTS + 1))

# 3. Plugin Memory Limit Test (256 MB ceiling)
echo -e "${BLUE}[3/6] Testing Plugin 256MB Linear Memory Ceiling...${NC}"
cargo test -p tempo-plugin --test plugin_tests -- plugin_memory_limit --nocapture
echo -e "${GREEN}✓ Plugin memory ceiling verified (Allocations >256MB rejected)${NC}\n"
PASSED_TESTS=$((PASSED_TESTS + 1))

# 4. Code Mode Lua Engine Test
echo -e "${BLUE}[4/6] Testing Code Mode Lua Scripting & Undo/Redo...${NC}"
cargo test -p tempo-plugin --test plugin_tests -- code_mode_lua --nocapture
echo -e "${GREEN}✓ Code Mode Lua scripting verified with undo-able commands${NC}\n"
PASSED_TESTS=$((PASSED_TESTS + 1))

# 5. Python Compute Server IPC Ping Test
echo -e "${BLUE}[5/6] Testing Python Compute Server JSON-RPC IPC...${NC}"
cargo test -p tempo-compute --test server_ping -- --nocapture
echo -e "${GREEN}✓ Python compute server IPC verified over Unix domain socket${NC}\n"
PASSED_TESTS=$((PASSED_TESTS + 1))

# 6. Plugin Effect Registration and Lifecycle Test
echo -e "${BLUE}[6/6] Testing Effect Plugin Registration & Lifecycle...${NC}"
cargo run -p tempo-app --example plugin_effect_test -- tests/plugins/test_effect_plugin.wasm
echo -e "${GREEN}✓ Plugin effect registration, shader binding, and cleanup verified${NC}\n"
PASSED_TESTS=$((PASSED_TESTS + 1))

echo -e "${GREEN}======================================================${NC}"
echo -e "${GREEN}All Phase 4 tests passed! (${PASSED_TESTS}/${TOTAL_TESTS})${NC}"
echo -e "${GREEN}======================================================${NC}"
