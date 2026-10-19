#!/usr/bin/env bash
# Maintainer-only: export an existing build, never evaluate or compile a flake.
set -euo pipefail
if (( $# != 2 )); then
  echo 'Usage: bash packaging/nix/export-prebuilt.sh /path/to/result /new/output/directory' >&2
  exit 2
fi
talis_scripts=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
talis_store=$(readlink -f -- "$1")
if [[ ! $talis_store =~ ^/nix/store/[0-9a-z]{32}-talis-([0-9][0-9A-Za-z.+-]*)$ || ! -x $talis_store/bin/talis ]]; then
  echo 'Expected an already-built Talis Nix store output.' >&2
  exit 1
fi
talis_name="talis-${BASH_REMATCH[1]}-nixos-x86_64"
talis_output=$2
if [[ $talis_output != /* || -e $talis_output ]]; then
  echo 'Choose a new, absolute output directory; existing output is never replaced.' >&2
  exit 1
fi
nix-store --check-validity "$talis_store"
mkdir -p -- "$talis_output/$talis_name"
talis_bundle="$talis_output/$talis_name"
cp -- "$talis_scripts/install-prebuilt.sh" "$talis_bundle/install.sh"
cp -- "$talis_scripts/PREBUILT.md" "$talis_bundle/README.md"
printf '%s\n' "$talis_store" > "$talis_bundle/store-path"
nix-store --query --requisites "$talis_store" > "$talis_bundle/store-paths"
mapfile -t talis_runtime_paths < "$talis_bundle/store-paths"
nix-store --export "${talis_runtime_paths[@]}" | gzip -1 > "$talis_bundle/talis.nar.gz"
(
  cd -- "$talis_bundle"
  sha256sum talis.nar.gz store-path store-paths install.sh README.md > SHA256SUMS
)
tar -czf "$talis_output/$talis_name.tar.gz" -C "$talis_output" "$talis_name"
(
  cd -- "$talis_output"
  sha256sum "$talis_name.tar.gz" > SHA256SUMS
)
echo "Prebuilt archive: $talis_output/$talis_name.tar.gz"
echo "Archive checksum: $talis_output/SHA256SUMS"
echo 'The expanded bundle is retained beside it for inspection and install testing.'
