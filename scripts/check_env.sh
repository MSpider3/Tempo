#!/usr/bin/env bash
# Checks that everything needed to build Tempo is installed.
set -e

need() { pkg-config --exists "$1" || { echo "MISSING: $1  ($2)"; missing=1; }; }
missing=0

cargo --version
need gtk4            "Fedora: gtk4-devel            Ubuntu: libgtk-4-dev"
need libadwaita-1    "Fedora: libadwaita-devel      Ubuntu: libadwaita-1-dev"
need libpipewire-0.3 "Fedora: pipewire-devel        Ubuntu: libpipewire-0.3-dev"
need libavcodec      "Fedora: ffmpeg-free-devel     Ubuntu: libavcodec-dev"
need libavformat     "Fedora: ffmpeg-free-devel     Ubuntu: libavformat-dev"
need libswscale      "Fedora: ffmpeg-free-devel     Ubuntu: libswscale-dev"
need libswresample   "Fedora: ffmpeg-free-devel     Ubuntu: libswresample-dev"

pkg-config --atleast-version=1.6 libadwaita-1 || { echo "libadwaita 1.6 or newer is required"; missing=1; }
command -v glib-compile-resources >/dev/null || { echo "MISSING: glib-compile-resources (glib2-devel / libglib2.0-dev-bin)"; missing=1; }
command -v ffmpeg >/dev/null || { echo "MISSING: ffmpeg program (needed at run time for export and proxies)"; missing=1; }

[ "$missing" = 0 ] && echo "Environment OK" || exit 1
