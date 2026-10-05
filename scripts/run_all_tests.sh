#!/usr/bin/env bash
# ==============================================================================
# Tempo Video Editor - Run All Automated Phase Test Suites
# Executes all automated phase test suites in sequence.
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

GREEN='\033[0;32m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

echo -e "${BOLD}${BLUE}======================================================${NC}"
echo -e "${BOLD}${BLUE}Tempo 2 Video Editor — Complete Test Suite Execution${NC}"
echo -e "${BOLD}${BLUE}======================================================${NC}\n"

# Step 0: Check Environment
echo -e "${BLUE}>>> Running Environment Check...${NC}"
./scripts/check_env.sh

# Phase 0: Foundations & Project Scaffolding
echo -e "\n${BLUE}>>> Running Phase 0 Test Suite...${NC}"
./scripts/test_phase0.sh

# Phase 1: Minimal Playback Engine
echo -e "\n${BLUE}>>> Running Phase 1 Test Suite...${NC}"
./scripts/test_phase1.sh

# Phase 2: Cut Page & Core Engines
echo -e "\n${BLUE}>>> Running Phase 2 Test Suite...${NC}"
./scripts/test_phase2.sh

# Phase 3: Edit Page & Advanced Editorial
echo -e "\n${BLUE}>>> Running Phase 3 Test Suite...${NC}"
./scripts/test_phase3.sh

# Phase 4: WASM Plugins & Python Compute Server
echo -e "\n${BLUE}>>> Running Phase 4 Test Suite...${NC}"
./scripts/test_phase4.sh

# Phase 5: Polish, Benchmarks & Release Regression
echo -e "\n${BLUE}>>> Running Phase 5 Test Suite...${NC}"
./scripts/test_phase5.sh

echo -e "\n${BOLD}${GREEN}======================================================${NC}"
echo -e "${BOLD}${GREEN}All Tempo Phase Test Suites (0 to 5) Passed Successfully!${NC}"
echo -e "${BOLD}${GREEN}======================================================${NC}"
