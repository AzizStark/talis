// Debian packaging only: tiny ELF fixtures, no Rust, GTK session or host install.
import assert from 'node:assert/strict';
import {mkdtempSync, mkdirSync, writeFileSync, readFileSync, copyFileSync, readdirSync, existsSync, statSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';

const temporary = mkdtempSync(join(tmpdir(), 'talis-debian-'));
try {
    const repo = join(temporary, "source with 'quote");
    const target = join(temporary, "build with 'quote");
    const commands = join(temporary, 'commands');
    const vendor = 'packaging/third-party/gtk4-layer-shell';
    for (const directory of ['debian','scripts',vendor,'assets/ui/branding','integrations/gnome'])
        mkdirSync(join(repo, directory), {recursive:true});
    mkdirSync(commands);
    for (const path of ['debian/build.sh','scripts/stage-release.sh','scripts/launch.sh','scripts/session.sh',
        `${vendor}/gtk4-layer-shell-1.3.0.tar.gz`,`${vendor}/SHA256SUMS`,`${vendor}/LICENSE`,`${vendor}/NOTICE`])
        copyFileSync(new URL(`../${path}`, import.meta.url), join(repo, path));
    for (const path of ['LICENSE','LICENSING.md','packaging/io.github.azizstark.Talis.desktop',
        'packaging/io.github.azizstark.Talis.metainfo.xml','assets/ui/branding/io.github.azizstark.Talis.svg',
        'assets/ui/branding/io.github.azizstark.TalisTray-symbolic.svg'])
        writeFileSync(join(repo, path), path);
    const companion = new URL('../integrations/gnome/', import.meta.url);
    for (const name of readdirSync(companion).filter(name => /\.(js|json)$/.test(name)))
        copyFileSync(new URL(name, companion), join(repo, 'integrations/gnome', name));
    const call = (command, args, changes={}) => spawnSync(command, args, {
        cwd:repo, encoding:'utf8', timeout:15000, env:{...process.env,...changes},
    });
    const ok = result => assert.equal(result.status, 0, result.stderr || result.stdout);
    const library = join(temporary, 'libgtk4-layer-shell.so.0');
    const wayland = join(temporary, 'wayland');
    writeFileSync(join(temporary, 'fixture.c'), 'int main(void) { return 0; }\n');
    ok(call('cc', ['-shared','-fPIC','-Wl,-soname,libgtk4-layer-shell.so.0',join(temporary,'fixture.c'),'-o',library]));
    ok(call('cc', [join(temporary,'fixture.c'),'-Wl,--no-as-needed',library,'-o',wayland]));
    const trace = join(temporary, 'cargo-trace');
    const mesonTrace = join(temporary, 'meson-trace');
    writeFileSync(join(commands,'pkg-config'), '#!/bin/sh\nexit 0\n', {mode:0o755});
    writeFileSync(join(commands,'meson'), '#!/bin/sh\nprintf "configured\\n" > "$TEST_MESON_TRACE"\nexit 23\n', {mode:0o755});
    writeFileSync(join(commands,'cargo'), `#!/usr/bin/env bash
set -euo pipefail
printf '%s|%s|%s|%s\\n' "$CARGO_TARGET_DIR" "\${PKG_CONFIG_PATH:-}" "\${LD_LIBRARY_PATH:-}" "$*" >> "$TEST_TRACE"
[[ \${TEST_FAIL_BUILD:-0} != 1 ]] || exit 19
mkdir -p "$CARGO_TARGET_DIR/release"
if [[ " $* " == *' --features wayland-desktop '* ]]; then
  cp "$TEST_WAYLAND" "$CARGO_TARGET_DIR/release/talis"
else
  cp /bin/true "$CARGO_TARGET_DIR/release/talis"
  cp /bin/true "$CARGO_TARGET_DIR/release/talis-gnome-scene"
fi
`, {mode:0o755});
    const environment = {PATH:`${commands}:/usr/bin:/bin`, PKG_CONFIG_PATH:'', LD_LIBRARY_PATH:'',
        TEST_TRACE:trace, TEST_MESON_TRACE:mesonTrace, TEST_WAYLAND:wayland};
    const action = (stage, changes={}) => call('bash', [join(repo,'debian/build.sh'),stage,target], {...environment,...changes});
    assert.notEqual(action('unknown').status, 0);
    assert.equal(action('configure').status, 23, 'verified source must reach Meson');
    assert(existsSync(mesonTrace));
    assert(!readdirSync(join(repo,'debian/.build')).some(name => name.startsWith('.gtk4-layer-shell.')),
        'failed configuration cleans temporary source/build trees');
    rmSync(mesonTrace);
    const archive = join(repo,vendor,'gtk4-layer-shell-1.3.0.tar.gz');
    const original = readFileSync(archive);
    writeFileSync(archive, Buffer.concat([original,Buffer.from('altered')]));
    assert.notEqual(action('configure').status, 0);
    assert(!existsSync(mesonTrace), 'reject altered vendored source before executing its build');
    writeFileSync(archive, original);
    ok(action('build'));
    ok(action('test'));
    const layer = join(repo,'debian/.build/layer-shell');
    const builds = readFileSync(trace, 'utf8');
    assert(builds.includes(`${target}/x11|||build --locked --release --bin talis --bin talis-gnome-scene`));
    assert(builds.includes(`${target}/wayland|${layer}/lib/pkgconfig|${layer}/lib|build --locked --release --features wayland-desktop --bin talis`));
    assert(builds.includes(`${target}/x11|||test --locked --release`));
    assert(builds.includes('test --locked --release --features wayland-desktop'));
    assert.equal(action('build', {TEST_FAIL_BUILD:'1'}).status, 19, 'propagate Rust build failure');
    const destination = join(repo,'debian/talis');
    assert.notEqual(action('install').status, 0, 'a missing private library must fail packaging');
    assert(!existsSync(destination), 'validate Wayland inputs before staging anything');
    mkdirSync(join(layer,'lib'), {recursive:true});
    copyFileSync(library, join(layer,'lib/libgtk4-layer-shell.so.0'));
    assert.notEqual(action('install').status, 0, 'license notices are required');
    assert(!existsSync(destination));
    writeFileSync(join(layer,'NOTICE'), 'Drew DeVault\nIsaac Freund\nKristian Høgsberg\n');
    ok(action('install'));
    for (const name of ['talis-bin','talis-wayland-bin','talis-gnome-scene'])
        assert.equal(statSync(join(destination,'usr/lib/talis',name)).mode & 0o777, 0o755);
    assert.deepEqual(readFileSync(join(destination,'usr/lib/talis/libgtk4-layer-shell.so.0')), readFileSync(library));
    assert(existsSync(join(destination,'usr/share/gnome-shell/extensions/talis@azizstark/extension.js')));
    for (const name of ['LICENSE','NOTICE'])
        assert(existsSync(join(destination,`usr/share/doc/talis/gtk4-layer-shell-${name}`)));
    for (const path of ['home','etc','debian','setup-gnome.sh','usr/share/talis/gnome','usr/lib/libgtk4-layer-shell.so.0'])
        assert(!existsSync(join(destination,path)), `Unexpected portable/user/system file: ${path}`);
    const rules = readFileSync(new URL('../debian/rules', import.meta.url),'utf8');
    for (const phase of ['configure','build','test','install'])
        assert(rules.includes(`bash debian/build.sh ${phase} "$(CARGO_TARGET_DIR)"`));
    assert.match(rules, /dh_makeshlibs -Xlibgtk4-layer-shell/);
    assert.match(rules, /dh_shlibdeps -l"\$\(CURDIR\)\/debian\/talis\/usr\/lib\/talis"/);
    assert(!rules.includes('--ignore-missing-info'));
    assert(readFileSync(new URL('../debian/clean',import.meta.url),'utf8').includes('debian/.build/'));
    console.log('PASS: Debian backend separation, checksum guard, build failures, private runtime, notices, clean scope and complete system companion');
} finally {
    rmSync(temporary, {recursive:true, force:true});
}
