#!/usr/bin/env bash
# Local script-control-flow fixture, not a Nix implementation or build test.
set -euo pipefail
case ${0##*/} in
  nix-store)
    case $1 in
      --check-validity) exit 0 ;;
      --query) printf '%s\n' "$TALIS_TEST_STORE" ;;
      --export) printf 'fake runtime export\n' ;;
      *) exit 1 ;;
    esac
    ;;
  nix)
    [[ $1 == --extra-experimental-features && $2 == 'nix-command flakes' ]]
    [[ $3 == --offline && $4 == --max-jobs && $5 == 0 ]]
    [[ $6 == --option && $7 == builders && -z $8 ]]
    [[ $9 == --option && ${10} == substitute && ${11} == false ]]
    shift 11
    case "$1 $2" in
      'profile list')
        [[ -f $TALIS_TEST_ROOT/installed ]] || exit 1
        printf '{"elements":{"talis":{"storePaths":["%s"]}}}\n' "$TALIS_TEST_STORE"
        ;;
      'eval --impure')
        [[ $TALIS_STORE_PATH == "$TALIS_TEST_STORE" && -n $TALIS_PROFILE_JSON ]]
        printf 'yes'
        ;;
      'profile install')
        [[ $3 == "$TALIS_TEST_STORE" ]]
        printf 'installed\n' >> "$TALIS_TEST_ROOT/installed"
        ;;
      *) exit 1 ;;
    esac
    ;;
  *) exit 1 ;;
esac
