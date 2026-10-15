#!/usr/bin/env bash
# Build and start the desktop app from source. Extra arguments configure that launch.
set -euo pipefail
talis_repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
talis_backend=${TALIS_BACKEND:-auto}
if [[ $talis_backend == auto ]]; then
  if [[ ${XDG_SESSION_TYPE:-} == wayland || (-n ${WAYLAND_DISPLAY:-} && ${XDG_SESSION_TYPE:-} != x11) ]]; then
    talis_backend=wayland
  else
    talis_backend=x11
  fi
fi
case "$talis_backend" in
  x11|wayland) ;;
  *) echo 'TALIS_BACKEND must be auto, x11 or wayland. GNOME Wayland uses its companion extension.' >&2; exit 2 ;;
esac
if [[ $talis_backend == wayland ]]; then
  talis_desktops=${XDG_CURRENT_DESKTOP:-}
  case ":${talis_desktops,,}:" in
    *:gnome:*) echo 'GNOME Wayland requires the companion extension in integrations/gnome. Build its client with: bash scripts/build.sh gnome' >&2; exit 1 ;;
  esac
fi
cd -- "$talis_repo"
talis_target=${CARGO_TARGET_DIR:-target}
if [[ $talis_target != /* ]]; then talis_target="$talis_repo/$talis_target"; fi
# Cargo tracks source, embedded assets and feature changes; unchanged builds are
# incremental. A failed build stops this script instead of launching an old app.
bash "$talis_repo/scripts/build.sh" "$talis_backend"
talis_args=(--desktop --arm --arm-flow --now-playing)
case "$talis_backend" in
  x11)
    # Presentation/backend policy lives in the app, shared with login startup.
    # Installed/autostart launches use the binary directly, without build tools.
    exec "$talis_target/release/talis" "${talis_args[@]}" "$@" ;;
  wayland)
    talis_layer_library=${TALIS_LAYER_SHELL_LIB:-}
    if [[ -z $talis_layer_library ]]; then
      for talis_library in /usr/lib/libgtk4-layer-shell.so.0 /usr/lib64/libgtk4-layer-shell.so.0 \
        /usr/lib/x86_64-linux-gnu/libgtk4-layer-shell.so.0 /usr/lib/aarch64-linux-gnu/libgtk4-layer-shell.so.0; do
        if [[ -f $talis_library ]]; then talis_layer_library=$talis_library; break; fi
      done
    fi
    if [[ -z $talis_layer_library ]] && command -v pkg-config >/dev/null && pkg-config --exists gtk4-layer-shell-0; then
      talis_layer_library="$(pkg-config --variable=libdir gtk4-layer-shell-0)/libgtk4-layer-shell.so.0"
    fi
    [[ -f $talis_layer_library ]] || { echo 'GTK4 layer-shell runtime is missing.' >&2; exit 1; }
    talis_layer_dir=$(dirname -- "$talis_layer_library")
    unset GDK_DEBUG GDK_DISABLE
    exec env GDK_BACKEND=wayland LD_LIBRARY_PATH="$talis_layer_dir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
      LD_PRELOAD="$talis_layer_library${LD_PRELOAD:+:$LD_PRELOAD}" \
      "$talis_target/release/talis" "${talis_args[@]}" --experimental-wayland "$@" ;;
esac
