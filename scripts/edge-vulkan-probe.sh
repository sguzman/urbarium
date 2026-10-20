#!/usr/bin/env bash
# Urbarium isolated Edge/Vulkan diagnostic: never edits the normal Edge profile.
set -euo pipefail

if command -v microsoft-edge-stable >/dev/null 2>&1; then
  edge_bin=$(command -v microsoft-edge-stable)
elif command -v microsoft-edge >/dev/null 2>&1; then
  edge_bin=$(command -v microsoft-edge)
elif [[ -x /opt/microsoft/msedge/microsoft-edge ]]; then
  edge_bin=/opt/microsoft/msedge/microsoft-edge
else
  printf 'Microsoft Edge not found. No changes were made.\n' >&2
  exit 1
fi

if [[ ! -e /dev/dri/renderD128 ]]; then
  printf 'Expected AMD render node /dev/dri/renderD128 not found. No changes were made.\n' >&2
  exit 1
fi

profile_root=${XDG_RUNTIME_DIR:-/tmp}
profile=$(mktemp -d "${profile_root%/}/urbarium-edge-vulkan.XXXXXXXX")
trap 'rm -rf -- "$profile"' EXIT

printf '\nUrbarium WebGPU hardware diagnostic (isolated Edge window)\n'
printf 'Dedicated temporary profile: %s\n' "$profile"
printf 'Normal Edge profile and flags are unchanged.\n'
printf 'In the window, click "Run adapter test and download JSON"; keep the JSON for diagnostics.\n'
printf 'The GPU test does not render a city or load your private browsing data.\n'
printf 'This test enables Chromium Vulkan in a separate Edge process; if unstable, close it.\n\n'

"$edge_bin" \
  --user-data-dir="$profile" \
  --no-first-run --no-default-browser-check \
  --ozone-platform=wayland \
  --render-node-override=/dev/dri/renderD128 \
  --enable-unsafe-webgpu \
  --enable-features=Vulkan \
  --new-window \
  'https://sguzman.github.io/urbarium/gpu-test.html?launch=isolated-vulkan'
