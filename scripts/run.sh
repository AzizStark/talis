#!/usr/bin/env bash
# Build and start the desktop app from source. Extra arguments configure that launch.
set -euo pipefail
talis_repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
source "$talis_repo/scripts/session.sh"
talis_backend=$(talis_session_backend)
if [[ $talis_backend == wayland ]] && talis_session_is_gnome; then
  echo 'GNOME Wayland requires the companion extension in integrations/gnome. Build its client with: bash scripts/build.sh gnome' >&2
  exit 1
fi
cd -- "$talis_repo"
talis_target=${CARGO_TARGET_DIR:-target}
if [[ $talis_target != /* ]]; then talis_target="$talis_repo/$talis_target"; fi
# Cargo tracks source, embedded assets and feature changes; unchanged builds are
# incremental. A failed build stops this script instead of launching an old app.
bash "$talis_repo/scripts/build.sh" "$talis_backend"
if [[ $talis_backend == wayland && -z ${TALIS_LAYER_SHELL_LIB:-} ]]; then
  talis_layer_directory=$(pkg-config --variable=libdir gtk4-layer-shell-0)
  if [[ -f $talis_layer_directory/libgtk4-layer-shell.so.0 ]]; then
    export TALIS_LAYER_SHELL_LIB="$talis_layer_directory/libgtk4-layer-shell.so.0"
  fi
fi
export TALIS_BINARY="$talis_target/release/talis" TALIS_DATA_DIR="$talis_repo" TALIS_BACKEND="$talis_backend"
exec bash "$talis_repo/scripts/launch.sh" "$@"
