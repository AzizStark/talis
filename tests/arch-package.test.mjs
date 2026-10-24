// Exercise the real PKGBUILD's package function without pacman or a host install.
import assert from 'node:assert/strict';
import {mkdtempSync, mkdirSync, writeFileSync, readFileSync, readdirSync, readlinkSync, statSync, existsSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';

const temporary = mkdtempSync(join(tmpdir(), 'talis-arch-'));
try {
    const source = join(temporary, "source with 'quote");
    const runtime = join(source, 'talis-linux-x86_64/usr');
    const destination = join(temporary, "package with 'quote");
    const files = [
        'lib/talis/talis-bin', 'lib/talis/talis-wayland-bin', 'lib/talis/talis-gnome-scene',
        'lib/talis/launch.sh', 'lib/talis/session.sh', 'lib/talis/libgtk4-layer-shell.so.0',
        'share/talis/assets/scenes/1080p/day/beauty.png',
        'share/applications/io.github.azizstark.Talis.desktop',
        'share/icons/hicolor/scalable/apps/io.github.azizstark.Talis.svg',
        'share/icons/hicolor/scalable/status/io.github.azizstark.TalisTray-symbolic.svg',
        'share/metainfo/io.github.azizstark.Talis.metainfo.xml',
        'share/doc/talis/LICENSE', 'share/doc/talis/LICENSING.md',
        'share/doc/talis/gtk4-layer-shell-LICENSE',
    ];
    const companion = new URL('../integrations/gnome/', import.meta.url);
    const companionFiles = readdirSync(companion).filter(name => /\.(js|json)$/.test(name));
    files.push(...[...companionFiles, 'LICENSE'].map(name => `share/talis/gnome/${name}`));
    files.push('share/talis/gnome/unwanted.blend');
    for (const name of files) {
        mkdirSync(join(runtime, name, '..'), {recursive:true});
        writeFileSync(join(runtime, name), name);
    }
    const recipe = fileURLToPath(new URL('../packaging/arch/PKGBUILD', import.meta.url));
    const result = spawnSync('bash', ['-euc', `
        source "$1"
        srcdir=$2
        pkgdir=$3
        declare -p pkgname pkgver pkgrel arch depends provides conflicts source sha256sums
        [[ ! -v install ]] # No install-time scripts or per-user settings mutations.
        package
    `, 'arch-test', recipe, source, destination], {encoding:'utf8', timeout:10000});
    assert.equal(result.status, 0, result.stderr);
    assert(result.stdout.includes('talis-bin') && result.stdout.includes('x86_64'));
    assert(result.stdout.includes('gtk4-layer-shell') && result.stdout.includes('glibc>=2.39'));
    const version = readFileSync(new URL('../Cargo.toml', import.meta.url), 'utf8')
        .match(/^version = "([^"]+)"$/m)[1];
    assert(result.stdout.includes(`talis-${version}-portable-linux-x86_64.tar.gz`));
    assert.match(result.stdout, /sha256sums=.*[a-f0-9]{64}/);
    for (const name of ['talis-bin','talis-wayland-bin','talis-gnome-scene','launch.sh']) {
        const path = join(destination, 'usr/lib/talis', name);
        assert.equal(readFileSync(path, 'utf8'), `lib/talis/${name}`);
        assert.equal(statSync(path).mode & 0o777, 0o755);
    }
    assert.equal(readlinkSync(join(destination, 'usr/bin/talis')), '../lib/talis/launch.sh');
    assert.equal(readlinkSync(join(destination, 'usr/bin/talis-gnome-scene')), '../lib/talis/talis-gnome-scene');
    const extension = join(destination, 'usr/share/gnome-shell/extensions/talis@azizstark');
    assert.deepEqual(readdirSync(extension).sort(), [...companionFiles, 'LICENSE'].sort());
    for (const name of companionFiles)
        assert.equal(statSync(join(extension, name)).mode & 0o777, 0o644);
    for (const path of ['usr/lib/talis/libgtk4-layer-shell.so.0', 'usr/share/talis/gnome',
        'usr/share/doc/talis/gtk4-layer-shell-LICENSE', 'etc', 'home', 'setup-gnome.sh', 'README.md'])
        assert(!existsSync(join(destination, path)), `Unexpected portable-only/user file: ${path}`);
    for (const name of ['LICENSE', 'LICENSING.md'])
        assert(existsSync(join(destination, 'usr/share/licenses/talis-bin', name)));
    assert(existsSync(join(destination, 'usr/share/talis/assets/scenes/1080p/day/beauty.png')));
    assert(existsSync(join(destination, 'usr/share/icons/hicolor/scalable/status/io.github.azizstark.TalisTray-symbolic.svg')));
    console.log('PASS: Arch release layout, both backends, system companion/library, notices, permissions, quoted paths and no install-time user mutations');
} finally {
    rmSync(temporary, {recursive:true, force:true});
}
