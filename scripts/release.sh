#!/usr/bin/env bash
# Build a complete portable release. Never installs or replaces existing output.
set -euo pipefail
if (($# != 2)); then
  echo 'Usage: bash scripts/release.sh OUTPUT_DIRECTORY LAYER_SHELL_LICENSE' >&2
  exit 2
fi
talis_repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
[[ $1 == /* && $1 != / && ! -e $1 && ! -L $1 ]] || {
  echo 'OUTPUT_DIRECTORY must be an absolute, unused directory.' >&2; exit 2;
}
[[ -f $2 ]] || { echo 'Provide the GTK4 layer-shell license file.' >&2; exit 1; }
[[ -z ${CARGO_BUILD_TARGET:-} ]] || { echo 'Cross-compilation is not supported by this release command.' >&2; exit 1; }
for talis_tool in tar gzip zip readelf sha256sum find sort xargs; do
  command -v "$talis_tool" >/dev/null || { echo "Missing release tool: $talis_tool" >&2; exit 1; }
done
bash "$talis_repo/scripts/check-deps.sh" wayland
talis_library=$(pkg-config --variable=libdir gtk4-layer-shell-0)/libgtk4-layer-shell.so.0
[[ -f $talis_library ]] || { echo 'The GTK4 layer-shell development package has no matching runtime library.' >&2; exit 1; }
talis_library=$(readlink -f -- "$talis_library")
talis_license=$(readlink -f -- "$2")
talis_output=$(realpath -m -- "$1")
cd -- "$talis_repo"
talis_version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' Cargo.toml)
[[ $talis_version =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][a-zA-Z0-9.+-]+)?$ ]] || {
  echo 'Cannot read the package version from Cargo.toml.' >&2; exit 1;
}
# Persistent, separate Cargo targets prevent one feature variant replacing the
# other. Cargo still checks freshness on every release, including dirty trees.
talis_target=${CARGO_TARGET_DIR:-$talis_repo/target}
[[ $talis_target == /* ]] || talis_target=$talis_repo/$talis_target
talis_target=$talis_target/packages
release_inputs() {
  {
    find Cargo.toml Cargo.lock LICENSE LICENSING.md src assets scripts packaging integrations/gnome -type f -print0 |
      sort -z | xargs -0 sha256sum --zero
    sha256sum --zero "$talis_library" "$talis_license"
  } | sha256sum
}
talis_before=$(release_inputs)
CARGO_TARGET_DIR="$talis_target/x11" bash scripts/build.sh gnome
CARGO_TARGET_DIR="$talis_target/wayland" bash scripts/build.sh wayland
mkdir -p -- "$(dirname -- "$talis_output")"
talis_temporary=$(mktemp -d "$(dirname -- "$talis_output")/.talis-release.XXXXXXXX")
trap 'rm -rf -- "$talis_temporary"' EXIT
talis_stage=$talis_temporary/talis-linux-x86_64
talis_artifacts=$talis_temporary/artifacts
mkdir -- "$talis_artifacts"
bash scripts/bundle.sh "$talis_stage" "$talis_target/x11/release" \
  "$talis_target/wayland/release/talis" "$talis_library" "$talis_license"
talis_archive=talis-$talis_version-portable-linux-x86_64.tar.gz
talis_extension=talis-gnome-$talis_version.shell-extension.zip
tar -C "$talis_temporary" -czf "$talis_artifacts/$talis_archive" talis-linux-x86_64
zip -q -j "$talis_artifacts/$talis_extension" integrations/gnome/*.js integrations/gnome/*.json LICENSE
[[ $(release_inputs) == "$talis_before" ]] || {
  echo 'Release inputs changed during the build; no artifacts published. Run again with a stable checkout.' >&2; exit 1;
}
(
  cd -- "$talis_artifacts"
  sha256sum "$talis_archive" "$talis_extension" > SHA256SUMS
)
# No-clobber also protects an output created while the build was in progress.
mv -T -n -- "$talis_artifacts" "$talis_output"
[[ ! -d $talis_artifacts ]] || { echo 'Output appeared during the build; nothing replaced.' >&2; exit 1; }
printf 'Release artifacts: %s\n' "$talis_output"
