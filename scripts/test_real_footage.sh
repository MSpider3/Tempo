#!/usr/bin/env bash
# ==============================================================================
# Tempo Video Editor - Real Footage Edit & Export Verification Test Script
# Tests cutting real footage to last 2 minutes, adding fade from black and
# fade to white transitions, and exporting to 720p 60fps with synchronized audio.
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
BOLD='\033[1m'
NC='\033[0m'

TARGET_DIR="/run/media/mehulgolecha/Extra Volume/SWSetup/Youcam/Setup/New folder/YT/NA"
OUTPUT_FILE="${TARGET_DIR}/test video.mp4"

echo -e "${BOLD}${BLUE}======================================================${NC}"
echo -e "${BOLD}${BLUE}Tempo 2 — Real Footage Edit & Export Test Suite${NC}"
echo -e "${BOLD}${BLUE}======================================================${NC}\n"

# Step 1: Run Rust Real Footage Editing Pipeline
echo -e "${BLUE}>>> Step 1: Executing Real Video Editing & Export in Tempo...${NC}"
cargo run -p tempo-app --example edit_real_footage

# Step 2: Verify Output File Exists and Has Non-Zero Size
echo -e "\n${BLUE}>>> Step 2: Checking Output File Integrity...${NC}"
if [[ ! -f "${OUTPUT_FILE}" ]]; then
    echo -e "${RED}FAIL: Output file does not exist at ${OUTPUT_FILE}${NC}"
    exit 1
fi

FILE_SIZE=$(stat -c%s "${OUTPUT_FILE}")
echo -e "${GREEN}✓ File exists: ${OUTPUT_FILE} (${FILE_SIZE} bytes)${NC}"

if [[ ${FILE_SIZE} -lt 1000000 ]]; then
    echo -e "${RED}FAIL: File size too small for 2-minute 720p60 video: ${FILE_SIZE} bytes${NC}"
    exit 1
fi

# Step 3: Probe Video and Audio Streams with ffprobe
echo -e "\n${BLUE}>>> Step 3: Validating Streams, Resolution, and Framerate with ffprobe...${NC}"
V_WIDTH=$(ffprobe -v error -select_streams v:0 -show_entries stream=width -of default=noprint_wrappers=1:nokey=1 "${OUTPUT_FILE}")
V_HEIGHT=$(ffprobe -v error -select_streams v:0 -show_entries stream=height -of default=noprint_wrappers=1:nokey=1 "${OUTPUT_FILE}")
V_R_FRAME_RATE=$(ffprobe -v error -select_streams v:0 -show_entries stream=r_frame_rate -of default=noprint_wrappers=1:nokey=1 "${OUTPUT_FILE}")
V_CODEC=$(ffprobe -v error -select_streams v:0 -show_entries stream=codec_name -of default=noprint_wrappers=1:nokey=1 "${OUTPUT_FILE}")
A_CODEC=$(ffprobe -v error -select_streams a:0 -show_entries stream=codec_name -of default=noprint_wrappers=1:nokey=1 "${OUTPUT_FILE}")
DURATION=$(ffprobe -v error -show_entries format=duration -of default=noprint_wrappers=1:nokey=1 "${OUTPUT_FILE}")

echo "  Video Codec:     ${V_CODEC} (Expected: h264)"
echo "  Audio Codec:     ${A_CODEC} (Expected: aac)"
echo "  Resolution:      ${V_WIDTH}x${V_HEIGHT} (Expected: 1280x720)"
echo "  Framerate:       ${V_R_FRAME_RATE} (Expected: 60/1)"
echo "  Duration:        ${DURATION}s (Expected: ~120.0s)"

if [[ "${V_WIDTH}" != "1280" || "${V_HEIGHT}" != "720" ]]; then
    echo -e "${RED}FAIL: Expected 1280x720 resolution, got ${V_WIDTH}x${V_HEIGHT}${NC}"
    exit 1
fi

if [[ "${V_R_FRAME_RATE}" != "60/1" ]]; then
    echo -e "${RED}FAIL: Expected 60/1 framerate, got ${V_R_FRAME_RATE}${NC}"
    exit 1
fi

if [[ "${V_CODEC}" != "h264" ]]; then
    echo -e "${RED}FAIL: Expected h264 video codec, got ${V_CODEC}${NC}"
    exit 1
fi

if [[ -z "${A_CODEC}" ]]; then
    echo -e "${RED}FAIL: Missing audio stream in exported video!${NC}"
    exit 1
fi

# Duration check within 1 second of 120.0s
DUR_INT=$(printf "%.0f" "${DURATION}")
if [[ ${DUR_INT} -lt 119 || ${DUR_INT} -gt 121 ]]; then
    echo -e "${RED}FAIL: Expected ~120s duration, got ${DURATION}s${NC}"
    exit 1
fi

echo -e "${GREEN}✓ All stream, resolution (720p), framerate (60fps), and duration (2 min) checks passed!${NC}"

# Step 4: Verify Transitions: Frame 0 (Black), Mid Frame (Video), Final Frame (White)
echo -e "\n${BLUE}>>> Step 4: Validating Visual Transitions (Start: Fade from Black, End: Fade to White)...${NC}"
TMP_DIR="/tmp/tempo_trans_test"
mkdir -p "${TMP_DIR}"

# Extract Start Frame (t = 0.0s)
ffmpeg -y -ss 0.0 -i "${OUTPUT_FILE}" -vframes 1 "${TMP_DIR}/start_frame.png" >/dev/null 2>&1
# Extract Mid Frame (t = 60.0s)
ffmpeg -y -ss 60.0 -i "${OUTPUT_FILE}" -vframes 1 "${TMP_DIR}/mid_frame.png" >/dev/null 2>&1
# Extract End Frame (t = 119.98s)
ffmpeg -y -sseof -0.1 -i "${OUTPUT_FILE}" -vframes 1 "${TMP_DIR}/end_frame.png" >/dev/null 2>&1

# Calculate average brightness using ffprobe signalstats
START_Y=$(ffmpeg -i "${TMP_DIR}/start_frame.png" -vf "signalstats,metadata=print:key=lavfi.signalstats.YAVG" -f null - 2>&1 | grep -o 'YAVG=[0-9.]*' | cut -d= -f2 | head -n1)
MID_Y=$(ffmpeg -i "${TMP_DIR}/mid_frame.png" -vf "signalstats,metadata=print:key=lavfi.signalstats.YAVG" -f null - 2>&1 | grep -o 'YAVG=[0-9.]*' | cut -d= -f2 | head -n1)
END_Y=$(ffmpeg -i "${TMP_DIR}/end_frame.png" -vf "signalstats,metadata=print:key=lavfi.signalstats.YAVG" -f null - 2>&1 | grep -o 'YAVG=[0-9.]*' | cut -d= -f2 | head -n1)

echo "  Start Frame Brightness (Yavg): ${START_Y} (Expected near 16 [black in studio range] or <= 20)"
echo "  Mid Frame Brightness (Yavg):   ${MID_Y} (Normal video content)"
echo "  End Frame Brightness (Yavg):   ${END_Y} (Expected near 235 [white in studio range] or >= 220)"

START_Y_INT=$(printf "%.0f" "${START_Y:-0}")
END_Y_INT=$(printf "%.0f" "${END_Y:-0}")

if [[ ${START_Y_INT} -gt 25 ]]; then
    echo -e "${RED}FAIL: Start frame is not black! Yavg=${START_Y}${NC}"
    exit 1
fi

if [[ ${END_Y_INT} -lt 210 ]]; then
    echo -e "${RED}FAIL: End frame is not white! Yavg=${END_Y}${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Visual transition verification PASSED: Fade-from-black start and Fade-to-white end confirmed!${NC}"

echo -e "\n${BOLD}${GREEN}======================================================${NC}"
echo -e "${BOLD}${GREEN}All Real Footage Editing & Export Verifications PASSED!${NC}"
echo -e "${BOLD}${GREEN}======================================================${NC}"
