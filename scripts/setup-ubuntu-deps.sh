#!/usr/bin/env bash
# Installs the build dependencies on Ubuntu 24.04. CI uses this script, so it is the list that is known to work.
set -euo pipefail

packages=(libwebkit2gtk-4.1-dev libasound2-dev clang cmake pkg-config
  ffmpeg libavcodec-dev libavformat-dev libavutil-dev libswscale-dev
  libswresample-dev libavfilter-dev libavdevice-dev libx264-dev
  patchelf libfuse2t64 librsvg2-bin libvulkan1 mesa-vulkan-drivers libvulkan-dev glslc)

if [[ ${GITHUB_ACTIONS:-} == true ]]; then
  # GitHub's Azure mirror sometimes serves a package every 10 s, and the install then took the job's whole hour.
  # The runner's ubuntu.sources reads its mirrors from apt-mirrors.txt, so each attempt pins one of them.
  for mirror in http://azure.archive.ubuntu.com/ubuntu/ https://archive.ubuntu.com/ubuntu/ https://security.ubuntu.com/ubuntu/; do
    printf '%s\n' "$mirror" | sudo tee /etc/apt/apt-mirrors.txt
    if sudo timeout --kill-after=10s 60s apt-get update && \
      sudo timeout --kill-after=10s 180s apt-get install -y --download-only "${packages[@]}"; then
      # Only downloads may be interrupted; dpkg runs without a timeout or further network access.
      sudo apt-get install -y --no-download "${packages[@]}"
      exit 0
    fi
    echo "::warning::Package download failed or timed out on $mirror"
  done
  echo '::error::Could not download Ubuntu dependencies from any of the three mirrors.'
  exit 1
fi

sudo apt-get update
sudo apt-get install -y "${packages[@]}"
