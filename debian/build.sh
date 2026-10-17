#!/usr/bin/env bash
# Debian build stages only; never installs into the running system.
set -euo pipefail
export LC_ALL=C
if (($# != 2)) || [[ $1 != configure && $1 != build && $1 != test && $1 != install ]]; then
  echo 'Usage: bash debian/build.sh configure|build|test|install BUILD_DIRECTORY' >&2
  exit 2
fi
talis_repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
talis_work=$(realpath -m -- "$2")
[[ $talis_work != / ]] || { echo 'Use a package build directory, not /.' >&2; exit 2; }
talis_layer=$talis_repo/debian/.build/layer-shell
talis_vendor=$talis_repo/packaging/third-party/gtk4-layer-shell
cd -- "$talis_repo"

with_wayland() {
  env CARGO_TARGET_DIR="$talis_work/wayland" \
    PKG_CONFIG_PATH="$talis_layer/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}" \
    LD_LIBRARY_PATH="$talis_layer/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" "$@"
}

case $1 in
  configure)
    pkg-config --atleast-version=4.8 gtk4
    # Verify before extraction or execution, even when build caches exist.
    (cd -- "$talis_vendor" && sha256sum --check SHA256SUMS)
    mkdir -p -- "$talis_repo/debian/.build"
    talis_temporary=$(mktemp -d "$talis_repo/debian/.build/.gtk4-layer-shell.XXXXXXXX")
    trap 'rm -rf -- "$talis_temporary"' EXIT
    mkdir -- "$talis_temporary/source"
    tar --extract --gzip --file "$talis_vendor/gtk4-layer-shell-1.3.0.tar.gz" \
      --directory "$talis_temporary/source" --strip-components=1 --no-same-owner
    cmp "$talis_vendor/LICENSE" "$talis_temporary/source/LICENSE"
    meson setup "$talis_temporary/build" "$talis_temporary/source" \
      --prefix="$talis_layer" --libdir=lib --buildtype=release --wrap-mode=nodownload \
      -Dintrospection=false -Dvapi=false -Dexamples=false -Dtests=false -Ddocs=false
    meson compile -C "$talis_temporary/build"
    meson install --no-rebuild -C "$talis_temporary/build"
    # Preserve notices for copied Wayland code and the protocols compiled here.
    talis_protocols=$(pkg-config --variable=pkgdatadir wayland-protocols)
    {
      cat "$talis_vendor/NOTICE"
      for talis_protocol in "$talis_temporary/source/protocol/wlr-layer-shell-unstable-v1.xml" \
        "$talis_protocols/stable/xdg-shell/xdg-shell.xml" \
        "$talis_protocols/staging/ext-session-lock/ext-session-lock-v1.xml"; do
        printf '\n%s\n\n' "$(basename -- "$talis_protocol")"
        sed -n '/<copyright>/,/<\/copyright>/{ /<copyright>/d; /<\/copyright>/d; p; }' "$talis_protocol"
      done
    } > "$talis_layer/NOTICE"
    ;;
  build)
    CARGO_TARGET_DIR="$talis_work/x11" cargo build --locked --release --bin talis --bin talis-gnome-scene
    with_wayland cargo build --locked --release --features wayland-desktop --bin talis
    ;;
  test)
    CARGO_TARGET_DIR="$talis_work/x11" cargo test --locked --release
    with_wayland cargo test --locked --release --features wayland-desktop
    ;;
  install)
    talis_wayland=$talis_work/wayland/release/talis
    talis_library=$talis_layer/lib/libgtk4-layer-shell.so.0
    # Prevent a successful-looking package that cannot select its Wayland backend.
    for talis_binary in "$talis_work/x11/release/talis" "$talis_work/x11/release/talis-gnome-scene" "$talis_wayland"; do
      [[ -x $talis_binary ]] || { echo "Missing executable: $talis_binary" >&2; exit 1; }
    done
    readelf -d "$talis_wayland" | grep -Fq '[libgtk4-layer-shell.so.0]'
    readelf -d "$talis_library" | grep -F '(SONAME)' | grep -Fq '[libgtk4-layer-shell.so.0]'
    [[ -s $talis_vendor/LICENSE && -s $talis_layer/NOTICE ]] || {
      echo 'GTK4 Layer Shell license notices are missing.' >&2; exit 1;
    }
    talis_machine=$(readelf -h "$talis_wayland" | sed -n 's/^[[:space:]]*Machine:[[:space:]]*//p')
    [[ -n $talis_machine ]] || { echo 'Cannot identify Wayland executable architecture.' >&2; exit 1; }
    for talis_binary in "$talis_work/x11/release/talis" "$talis_work/x11/release/talis-gnome-scene" "$talis_library"; do
      [[ $(readelf -h "$talis_binary" | sed -n 's/^[[:space:]]*Machine:[[:space:]]*//p') == "$talis_machine" ]] || {
        echo "Mismatched ELF architecture: $talis_binary" >&2; exit 1;
      }
    done
    for talis_binary in "$talis_work/x11/release/talis" "$talis_work/x11/release/talis-gnome-scene"; do
      if readelf -d "$talis_binary" | grep -Fq '[libgtk4-layer-shell.so.0]'; then
        echo 'X11 and GNOME executables must remain independent of layer-shell.' >&2; exit 1
      fi
    done
    talis_stage=$talis_repo/debian/talis
    bash scripts/stage-release.sh "$talis_stage" "$talis_work/x11/release" --with-gnome-extension
    install -m755 "$talis_wayland" "$talis_stage/usr/lib/talis/talis-wayland-bin"
    install -m644 "$talis_library" "$talis_stage/usr/lib/talis/libgtk4-layer-shell.so.0"
    install -m644 "$talis_vendor/LICENSE" "$talis_stage/usr/share/doc/talis/gtk4-layer-shell-LICENSE"
    install -m644 "$talis_layer/NOTICE" "$talis_stage/usr/share/doc/talis/gtk4-layer-shell-NOTICE"
    ;;
esac
