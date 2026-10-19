#!/usr/bin/env bash
# Local script-control-flow fixture, not a Nix implementation or build test.
set -euo pipefail
case ${0##*/} in
  nix-store)
    if [[ $1 == --max-jobs ]]; then
      [[ $2 == 0 && $3 == --option && $4 == builders && -z $5 ]]
      [[ $6 == --option && $7 == substitute && $8 == true ]]
      [[ $9 == --option && ${10} == fallback && ${11} == false ]]
      [[ ${12} == --option && ${13} == substituters && ${14} == https://cache.nixos.org ]]
      [[ ${15} == --option && ${16} == require-sigs && ${17} == true ]]
      shift 17
    fi
    case $1 in
      --check-validity)
        shift
        for path in "$@"; do
          if [[ $path == "$TALIS_TEST_DEPENDENCY" ]]; then
            [[ -f $TALIS_TEST_ROOT/dependency-ready ]] || exit 1
          elif [[ $path == "$TALIS_TEST_STORE" ]]; then
            [[ -f $TALIS_TEST_ROOT/payload-ready ]] || exit 1
          else
            exit 1
          fi
        done
        ;;
      --query) printf '%s\n' "$TALIS_TEST_STORE" "$TALIS_TEST_DEPENDENCY" ;;
      --export)
        [[ $# == 2 && $2 == "$TALIS_TEST_STORE" ]]
        printf 'fake Talis-only export\n'
        ;;
      --realise)
        [[ $# == 2 && $2 == "$TALIS_TEST_DEPENDENCY" ]]
        [[ ${TALIS_TEST_FAIL_CACHE:-0} != 1 ]] || exit 1
        printf 'downloaded\n' >> "$TALIS_TEST_ROOT/downloads"
        touch "$TALIS_TEST_ROOT/dependency-ready"
        ;;
      --import)
        [[ -f $TALIS_TEST_ROOT/dependency-ready ]]
        test "$(cat)" == 'fake Talis-only export'
        touch "$TALIS_TEST_ROOT/payload-ready"
        ;;
      *) exit 1 ;;
    esac
    ;;
  nix)
    [[ $1 == --extra-experimental-features && $2 == 'nix-command flakes' ]]
    shift 2
    [[ $1 == --max-jobs && $2 == 0 && $3 == --option && $4 == builders && -z $5 ]]
    shift 5
    if [[ $1 == store && $2 == copy-sigs ]]; then
      [[ $3 == --substituter && $4 == https://cache.nixos.org && $5 == --recursive && $6 == "$TALIS_TEST_STORE" ]]
      exit
    fi
    case $1 in
      path-info)
        if [[ $2 == --store ]]; then
          [[ $3 == https://cache.nixos.org && $5 == "$TALIS_TEST_DEPENDENCY" ]]
        fi
        printf '{}\n'
        exit
        ;;
      eval)
        [[ $2 == --impure && $3 == --raw && $4 == --file && -f $TALIS_NIX_PATH_INFO ]]
        printf 'payload %s\nsubstitute %s\n' "$TALIS_TEST_STORE" "$TALIS_TEST_DEPENDENCY"
        exit
        ;;
    esac
    [[ $1 == --option && $2 == substitute && $3 == true ]]
    [[ $4 == --option && $5 == fallback && $6 == false ]]
    shift 6
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
        [[ $3 == "$TALIS_TEST_STORE" && -f $TALIS_TEST_ROOT/payload-ready ]]
        printf 'installed\n' >> "$TALIS_TEST_ROOT/installed"
        ;;
      *) exit 1 ;;
    esac
    ;;
  *) exit 1 ;;
esac
