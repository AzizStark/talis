#!/usr/bin/env bash
# Install prebuilt Talis; download missing signed dependencies, never build.
set -euo pipefail
talis_bundle=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
if [[ $(uname -s) != Linux || $(uname -m) != x86_64 ]]; then
  echo 'This prebuilt package contains x86-64 Linux binaries.' >&2
  exit 1
fi
if (( EUID == 0 )); then
  echo 'Run this script as your normal user, without sudo. It will request sudo only for importing into the shared Nix store.' >&2
  exit 1
fi
for talis_tool in nix nix-store gzip sha256sum; do
  command -v "$talis_tool" >/dev/null || { echo "Required command is missing: $talis_tool" >&2; exit 1; }
done
cd -- "$talis_bundle"
sha256sum --check SHA256SUMS
IFS= read -r talis_store < store-path
if [[ ! $talis_store =~ ^/nix/store/[0-9a-z]{32}-talis-[0-9][0-9A-Za-z.+-]*$ ]]; then
  echo 'Invalid Talis store path in this bundle.' >&2
  exit 1
fi
talis_nix_store=$(command -v nix-store)
mapfile -t talis_runtime_paths < store-paths
mapfile -t talis_payload_paths < payload-paths
mapfile -t talis_substitute_paths < substitute-paths
declare -A talis_manifest=() talis_classified=()
talis_root_listed=false
for talis_runtime_path in "${talis_runtime_paths[@]}"; do
  [[ $talis_runtime_path =~ ^/nix/store/[0-9a-z]{32}-[0-9A-Za-z+._?=-]+$ && $talis_runtime_path != *.drv ]] || { echo 'Invalid runtime store path.' >&2; exit 1; }
  [[ ! -v talis_manifest[$talis_runtime_path] ]] || { echo 'Duplicate runtime store path.' >&2; exit 1; }
  talis_manifest[$talis_runtime_path]=1
  if [[ $talis_runtime_path == "$talis_store" ]]; then talis_root_listed=true; fi
done
[[ $talis_root_listed == true ]] || { echo 'Runtime manifest does not contain Talis.' >&2; exit 1; }
for talis_runtime_path in "${talis_payload_paths[@]}" "${talis_substitute_paths[@]}"; do
  [[ $talis_runtime_path =~ ^/nix/store/[0-9a-z]{32}-[0-9A-Za-z+._?=-]+$ && $talis_runtime_path != *.drv ]] || { echo 'Invalid classified store path.' >&2; exit 1; }
  [[ -v talis_manifest[$talis_runtime_path] && ! -v talis_classified[$talis_runtime_path] ]] || { echo 'Invalid or duplicate payload/dependency classification.' >&2; exit 1; }
  talis_classified[$talis_runtime_path]=1
done
[[ ${#talis_classified[@]} == ${#talis_manifest[@]} ]] || { echo 'Incomplete dependency manifest.' >&2; exit 1; }
printf '%s\n' "${talis_payload_paths[@]}" | grep -Fx -- "$talis_store" >/dev/null || { echo 'Talis is missing from the bundled payload.' >&2; exit 1; }
talis_download_options=(--max-jobs 0 --option builders '' --option substitute true --option fallback false --option substituters https://cache.nixos.org --option require-sigs true)
if (( ${#talis_substitute_paths[@]} > 0 )) && ! "$talis_nix_store" --check-validity "${talis_substitute_paths[@]}" 2>/dev/null; then
  echo 'Downloading missing prebuilt dependencies (no compiling)...'
  if ! "$talis_nix_store" "${talis_download_options[@]}" --realise "${talis_substitute_paths[@]}" >/dev/null; then
    echo 'Could not download required prebuilt dependencies. Check your connection and cache.nixos.org availability, then retry. No compilation was attempted.' >&2
    exit 1
  fi
fi
if ! "$talis_nix_store" --check-validity "${talis_payload_paths[@]}" 2>/dev/null; then
  echo 'Importing the prebuilt Talis application and scenes...'
  if [[ -w /nix/store && -w /nix/var/nix/db/db.sqlite ]]; then
    # Single-user Nix or an isolated store owned by this user.
    gzip -dc -- talis.nar.gz | "$talis_nix_store" --import
  else
    command -v sudo >/dev/null || { echo 'Import requires access to the shared Nix store; sudo is not available.' >&2; exit 1; }
    sudo -v
    gzip -dc -- talis.nar.gz | sudo "$talis_nix_store" --import
  fi
fi
nix-store --check-validity "${talis_runtime_paths[@]}"
echo 'Installing into your user profile (builders disabled)...'
talis_nix=(nix --extra-experimental-features 'nix-command flakes' --max-jobs 0 --option builders '' --option substitute true --option fallback false)
talis_installed=no
if talis_profile_json=$("${talis_nix[@]}" profile list --json 2>/dev/null); then
  # Parse only local profile metadata, without fetching/evaluating a source flake.
  talis_installed=$(TALIS_PROFILE_JSON="$talis_profile_json" TALIS_STORE_PATH="$talis_store" "${talis_nix[@]}" eval --impure --raw --expr '
    let
      profile = builtins.fromJSON (builtins.getEnv "TALIS_PROFILE_JSON");
      storePath = builtins.getEnv "TALIS_STORE_PATH";
      elements = profile.elements or [];
      entries = if builtins.isList elements then elements else builtins.attrValues elements;
    in if builtins.any (entry: (entry.active or true) && builtins.elem storePath (entry.storePaths or [])) entries
      then "yes" else "no"
  ')
fi
if [[ $talis_installed == yes ]]; then
  echo 'This Talis build is already installed; keeping the existing profile entry.'
elif ! "${talis_nix[@]}" profile install "$talis_store"; then
  echo 'Profile installation failed. No compilation was attempted. If an older Talis is already installed, check nix profile list for the conflicting entry before retrying.' >&2
  exit 1
fi
echo 'Installed. Quit any older Talis instance, then open Talis from the application menu or run: talis'
echo 'If startup was enabled with an older package, turn Run on startup off and on once.'
