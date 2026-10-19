#!/usr/bin/env bash
# Exercise export/install script control flow with fake Nix commands in isolation.
# Real store imports, profile evaluation and desktop tests are separate checks.
set -euo pipefail
talis_scripts=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
if [[ ${1:-} != --inside ]]; then
  talis_test=$(mktemp -d)
  trap 'rm -rf -- "$talis_test"' EXIT
  mkdir -p "$talis_test/store" "$talis_test/bin"
  cp -- "$talis_scripts/tests/fake-nix.sh" "$talis_test/bin/nix"
  chmod +x "$talis_test/bin/nix"
  ln -s nix "$talis_test/bin/nix-store"
  bwrap --tmpfs / --ro-bind /usr /usr --ro-bind /bin /bin \
    --ro-bind /lib /lib --ro-bind /lib64 /lib64 --ro-bind /etc /etc \
    --ro-bind /home /home --dev /dev --proc /proc --tmpfs /tmp \
    --bind "$talis_test/store" /nix/store --bind "$talis_test" "$talis_test" \
    --setenv TALIS_TEST_ROOT "$talis_test" \
    --setenv PATH "$talis_test/bin:/usr/bin:/bin" \
    bash "$talis_scripts/tests/prebuilt.sh" --inside
  exit
fi

mkdir -p /nix/var/nix/db
touch /nix/var/nix/db/db.sqlite
export TALIS_TEST_DEPENDENCY=/nix/store/11111111111111111111111111111111-test-library-1.0
for talis_version in 0.1.0 1.2.3-rc.1; do
  export TALIS_TEST_STORE="/nix/store/00000000000000000000000000000000-talis-$talis_version"
  mkdir -p "$TALIS_TEST_STORE/bin"
  cp -- /bin/true "$TALIS_TEST_STORE/bin/talis"
  touch "$TALIS_TEST_ROOT/dependency-ready" "$TALIS_TEST_ROOT/payload-ready"
  talis_output="$TALIS_TEST_ROOT/output-$talis_version"
  bash "$talis_scripts/export-prebuilt.sh" "$TALIS_TEST_STORE" "$talis_output"
  (cd -- "$talis_output" && sha256sum --check SHA256SUMS)
  talis_bundle="$talis_output/talis-$talis_version-nixos-x86_64"
  (cd -- "$talis_bundle" && sha256sum --check SHA256SUMS)
  tar -tzf "$talis_bundle.tar.gz" | grep '/install.sh$' >/dev/null
  if bash "$talis_scripts/export-prebuilt.sh" "$TALIS_TEST_STORE" "$talis_output"; then
    echo 'Exporter overwrote an existing output directory.' >&2
    exit 1
  fi
  # Installer must not add duplicate profile entries on a retry.
  rm -f -- "$TALIS_TEST_ROOT/installed" "$TALIS_TEST_ROOT/dependency-ready" "$TALIS_TEST_ROOT/payload-ready" "$TALIS_TEST_ROOT/downloads"
  if TALIS_TEST_FAIL_CACHE=1 bash "$talis_bundle/install.sh"; then
    echo 'Installer ignored a cache download failure.' >&2
    exit 1
  fi
  [[ ! -f $TALIS_TEST_ROOT/installed && ! -f $TALIS_TEST_ROOT/payload-ready ]]
  bash "$talis_bundle/install.sh"
  bash "$talis_bundle/install.sh"
  [[ $(wc -l < "$TALIS_TEST_ROOT/installed") == 1 ]]
  [[ $(wc -l < "$TALIS_TEST_ROOT/downloads") == 1 ]]
  # Recompute checksums so this exercises path validation, not tamper detection.
  cp -- "$talis_bundle/store-path" "$TALIS_TEST_ROOT/original-store-path"
  printf '/nix/store/00000000000000000000000000000000-unrelated-1.0\n' > "$talis_bundle/store-path"
  (cd -- "$talis_bundle" && sha256sum talis.nar.gz store-path store-paths payload-paths substitute-paths install.sh README.md > SHA256SUMS)
  if bash "$talis_bundle/install.sh"; then
    echo 'Installer accepted an unrelated store output.' >&2
    exit 1
  fi
  cp -- "$TALIS_TEST_ROOT/original-store-path" "$talis_bundle/store-path"
  (cd -- "$talis_bundle" && sha256sum talis.nar.gz store-path store-paths payload-paths substitute-paths install.sh README.md > SHA256SUMS)
  printf 'tampered\n' >> "$talis_bundle/README.md"
  if bash "$talis_bundle/install.sh"; then
    echo 'Installer accepted a bundle with a bad checksum.' >&2
    exit 1
  fi
done
echo 'Prebuilt script checks passed (mock Nix; no real store changes).'
