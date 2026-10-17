#!/usr/bin/env bash
# Isolated real Urbarium 3D WebGPU hardware test on AMD integrated GPU.
# No edits to the normal Edge profile, system settings, or Hyprland configuration.
# Reuse the Wayland + Vulkan path that previously returned AMD RDNA-2 in the
# real user's Edge adapter diagnostic. Do not assume Jobflick exports DISPLAY.
set -euo pipefail

if command -v microsoft-edge-stable >/dev/null 2>&1; then
  edge_bin=$(command -v microsoft-edge-stable)
elif command -v microsoft-edge >/dev/null 2>&1; then
  edge_bin=$(command -v microsoft-edge)
elif [[ -x /opt/microsoft/msedge/microsoft-edge ]]; then
  edge_bin=/opt/microsoft/msedge/microsoft-edge
else
  printf 'Microsoft Edge not found. No changes made.\n' >&2
  exit 1
fi

if [[ ! -e /dev/dri/renderD128 ]]; then
  printf 'Expected AMD iGPU render node /dev/dri/renderD128 not found. Aborting.\n' >&2
  exit 1
fi

if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
  printf 'WAYLAND_DISPLAY not set. A Wayland graphical session is required. No browser launched.\n' >&2
  exit 1
fi

profile_root=${XDG_RUNTIME_DIR:-/tmp}
profile=$(mktemp -d "${profile_root%/}/urbarium-wayland-vulkan-render.XXXXXXXX")
trap 'rm -rf -- "$profile"' EXIT

printf 'Urbarium real WebGPU rendering test: AMD iGPU / Wayland / Vulkan\n'
printf 'Temporary isolated Edge profile: %s\n' "$profile"
printf 'Normal Edge, Hyprland and GPU settings remain unchanged.\n'
printf 'Wait around 15 seconds, then click Download 3D rendering report.\n'
printf 'Upload urbarium-render-report-*.json to ChatGPT.\n'
printf 'Edge may print a Vulkan/Wayland incompatibility warning even when adapter discovery succeeds.\n'
printf 'If unstable or hot, immediately close this Edge test window.\n\n'

"$edge_bin" \
  --user-data-dir="$profile" \
  --no-first-run --no-default-browser-check \
  --ozone-platform=wayland \
  --render-node-override=/dev/dri/renderD128 \
  --enable-unsafe-webgpu \
  --enable-features=Vulkan \
  --new-window \
  'https://sguzman.github.io/urbarium/?renderer=webgpu&quality=low&launch=wayland-vulkan-render'
