#!/usr/bin/env bash
# Runtime launcher, shared by source and installed launches. Never builds.
set -euo pipefail
talis_launcher=$(readlink -f -- "${BASH_SOURCE[0]}")
talis_library_dir=$(dirname -- "$talis_launcher")
talis_binary=${TALIS_BINARY:-$talis_library_dir/talis-bin}
export TALIS_DATA_DIR=${TALIS_DATA_DIR:-$(cd -- "$talis_library_dir/../../share/talis" && pwd)}
source "$talis_library_dir/session.sh"
talis_backend=$(talis_session_backend)
talis_args=(--desktop --arm --arm-flow --now-playing)
if [[ $talis_backend == x11 ]]; then
  exec "$talis_binary" "${talis_args[@]}" "$@"
fi
if talis_session_is_gnome; then
    if command -v gdbus >/dev/null && gdbus call --session --timeout 5 \
      --dest org.gnome.Shell --object-path /io/github/azizstark/Talis \
      --method io.github.azizstark.Talis.OpenSettings >/dev/null 2>&1; then
      exit 0
    fi
    echo 'GNOME Wayland uses the Talis companion extension (GNOME Shell 48 or 50).' >&2
    echo 'For a portable bundle, run ./setup-gnome.sh once; then follow its enable instructions.' >&2
    exit 1
fi
# Portable bundles carry a separate feature-enabled executable. X11 packages
# remain independent of layer-shell, and source launches keep their override.
if [[ -z ${TALIS_BINARY:-} && -x $talis_library_dir/talis-wayland-bin ]]; then
  talis_binary=$talis_library_dir/talis-wayland-bin
fi
talis_layer_library=${TALIS_LAYER_SHELL_LIB:-}
if [[ -z $talis_layer_library ]]; then
  for talis_candidate in "$talis_library_dir/libgtk4-layer-shell.so.0" \
    /usr/lib/libgtk4-layer-shell.so.0 /usr/lib64/libgtk4-layer-shell.so.0 \
    /usr/lib/x86_64-linux-gnu/libgtk4-layer-shell.so.0 /usr/lib/aarch64-linux-gnu/libgtk4-layer-shell.so.0; do
    if [[ -f $talis_candidate ]]; then talis_layer_library=$talis_candidate; break; fi
  done
fi
[[ -f $talis_layer_library ]] || { echo 'GTK4 layer-shell runtime is missing; this package requires a Wayland-enabled build for this session.' >&2; exit 1; }
talis_layer_dir=$(dirname -- "$talis_layer_library")
talis_layer_name=$(basename -- "$talis_layer_library")
# LD_PRELOAD splits absolute paths on spaces. A soname plus a search directory
# supports relocated bundles in paths containing spaces without shell evaluation.
[[ $talis_layer_dir != *:* && $talis_layer_name != *[[:space:]:]* ]] || {
  echo 'The layer-shell directory cannot contain colons, or its filename whitespace.' >&2; exit 1;
}
unset GDK_DEBUG GDK_DISABLE
exec env GDK_BACKEND=wayland LD_LIBRARY_PATH="$talis_layer_dir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
  LD_PRELOAD="$talis_layer_name${LD_PRELOAD:+:$LD_PRELOAD}" \
  "$talis_binary" "${talis_args[@]}" --experimental-wayland "$@"
