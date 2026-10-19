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
talis_metadata=$(mktemp)
trap 'rm -f -- "$talis_metadata"' EXIT
talis_nix=(nix --extra-experimental-features 'nix-command flakes' --max-jobs 0 --option builders '')
# Legacy Nix exports omit signatures. Restore matching official signatures so
# re-exporting an imported release does not accidentally bundle every library.
"${talis_nix[@]}" store copy-sigs --substituter https://cache.nixos.org --recursive "$talis_store"
"${talis_nix[@]}" path-info --recursive --json "$talis_store" > "$talis_metadata"
touch "$talis_bundle/payload-paths" "$talis_bundle/substitute-paths"
talis_classification=$(TALIS_NIX_PATH_INFO="$talis_metadata" "${talis_nix[@]}" eval --impure --raw --file "$talis_scripts/split-store.nix")
while read -r talis_kind talis_path; do
  case $talis_kind in
    payload) printf '%s\n' "$talis_path" >> "$talis_bundle/payload-paths" ;;
    substitute) printf '%s\n' "$talis_path" >> "$talis_bundle/substitute-paths" ;;
    *) echo 'Unexpected path-info classification.' >&2; exit 1 ;;
  esac
done <<< "$talis_classification"
mapfile -t talis_payload_paths < "$talis_bundle/payload-paths"
mapfile -t talis_substitute_paths < "$talis_bundle/substitute-paths"
(( ${#talis_payload_paths[@]} > 0 )) || { echo 'No Talis payload was found.' >&2; exit 1; }
if (( ${#talis_substitute_paths[@]} > 0 )); then
  # Check availability before omitting signed dependencies from the archive.
  "${talis_nix[@]}" path-info --store https://cache.nixos.org --json "${talis_substitute_paths[@]}" >/dev/null
fi
nix-store --export "${talis_payload_paths[@]}" | gzip -1 > "$talis_bundle/talis.nar.gz"
(
  cd -- "$talis_bundle"
  sha256sum talis.nar.gz store-path store-paths payload-paths substitute-paths install.sh README.md > SHA256SUMS
)
tar -czf "$talis_output/$talis_name.tar.gz" -C "$talis_output" "$talis_name"
(
  cd -- "$talis_output"
  sha256sum "$talis_name.tar.gz" > SHA256SUMS
)
echo "Prebuilt archive: $talis_output/$talis_name.tar.gz"
echo "Archive checksum: $talis_output/SHA256SUMS"
echo "Bundled ${#talis_payload_paths[@]} Talis store entries; ${#talis_substitute_paths[@]} dependencies will be reused or downloaded."
echo 'The expanded bundle is retained beside it for inspection and install testing.'
