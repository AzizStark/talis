#!/usr/bin/env bash
# Import a locally transferred, checksum-verified Nix runtime closure. Never build.
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
talis_root_listed=false
for talis_runtime_path in "${talis_runtime_paths[@]}"; do
  [[ $talis_runtime_path =~ ^/nix/store/[0-9a-z]{32}-[^/[:space:]]+$ ]] || { echo 'Invalid runtime store path.' >&2; exit 1; }
  if [[ $talis_runtime_path == "$talis_store" ]]; then talis_root_listed=true; fi
done
[[ $talis_root_listed == true ]] || { echo 'Runtime manifest does not contain Talis.' >&2; exit 1; }
if ! nix-store --check-validity "${talis_runtime_paths[@]}" 2>/dev/null; then
  echo 'Importing the prebuilt runtime (no compiling)...'
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
echo 'Installing into your user profile (offline; builders disabled)...'
talis_nix=(nix --extra-experimental-features 'nix-command flakes' --offline --max-jobs 0 --option builders '' --option substitute false)
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
echo 'For the startup fix, turn Run on startup off and on once, then reboot to check the icons.'
