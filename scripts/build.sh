#!/usr/bin/env bash
set -euo pipefail
talis_repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
talis_backend=${1:-x11}
if (($# > 1)); then echo 'Usage: bash scripts/build.sh [x11|wayland|gnome]' >&2; exit 2; fi
bash "$talis_repo/scripts/check-deps.sh" "$talis_backend"
cd -- "$talis_repo"
case "$talis_backend" in
  x11) cargo build --locked --release --bin talis ;;
  wayland) cargo build --locked --release --features wayland-desktop --bin talis ;;
  gnome) cargo build --locked --release --bin talis --bin talis-gnome-scene ;;
esac
