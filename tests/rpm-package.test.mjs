// Exercise the RPM install layout without an RPM toolchain or host installation.
import assert from 'node:assert/strict';
import {mkdtempSync, mkdirSync, writeFileSync, readFileSync, readdirSync, readlinkSync, statSync, existsSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';

const temporary = mkdtempSync(join(tmpdir(), 'talis-rpm-'));
try {
    const source = join(temporary, "source with 'quote");
    const runtime = join(source, 'usr');
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
    const spec = readFileSync(new URL('../packaging/rpm/talis.spec', import.meta.url), 'utf8');
    assert.match(spec, /^ExclusiveArch:\s+x86_64$/m);
    assert.match(spec, /^Source0:\s+talis-%\{version\}-portable-linux-x86_64\.tar\.gz$/m);
    assert.match(spec, /^Requires:\s+gtk4 >= 4\.8$/m);
    assert.match(spec, /^Requires:\s+gtk4-layer-shell$/m);
    assert.match(spec, /^Vendor:\s+AzizStark$/m);
    assert.match(spec, /^Packager:\s+AzizStark <theazizstark@gmail\.com>$/m);
    assert(!/^%(?:pre|post|preun|postun|trigger\w*)(?:\s|$)/m.test(spec), 'No install-time settings mutations');
    assert(!/^AutoReq(?:Prov)?:\s*(?:no|0)/mi.test(spec), 'ELF dependencies must remain automatic');
    const prep = spec.split('\n%prep\n')[1].split('\n%build\n')[0];
    assert.match(prep, /[a-f0-9]{64}  %\{SOURCE0\}.*sha256sum --check --status/);
    assert(prep.indexOf('sha256sum') < prep.indexOf('%setup'), 'Verify before extracting');
    const install = spec.split('\n%install\n')[1].split('\n%check\n')[0];
    const macros = {buildroot:destination, _bindir:'/usr/bin', _datadir:'/usr/share', _metainfodir:'/usr/share/metainfo'};
    // Feed paths as quoted shell variables, not executable source text.
    const script = install.replace(/%\{(\w+)\}/g, (_, name) => {
        assert(name in macros, `Unexpected macro ${name}`);
        return name === 'buildroot' ? '${RPM_TEST_DEST}' : macros[name];
    });
    const result = spawnSync('bash', ['-euc', script], {
        cwd:source, encoding:'utf8', timeout:10000,
        env:{...process.env, RPM_TEST_DEST:destination},
    });
    assert.equal(result.status, 0, result.stderr);
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
    for (const path of ['usr/lib/talis/libgtk4-layer-shell.so.0','usr/share/talis/gnome',
        'usr/share/doc/talis/gtk4-layer-shell-LICENSE','etc','home','setup-gnome.sh','README.md'])
        assert(!existsSync(join(destination, path)), `Unexpected portable/user file: ${path}`);
    assert.match(spec, /^%license usr\/share\/doc\/talis\/LICENSE usr\/share\/doc\/talis\/LICENSING\.md$/m);
    assert(existsSync(join(destination, 'usr/share/talis/assets/scenes/1080p/day/beauty.png')));
    const trayIcon = 'usr/share/icons/hicolor/scalable/status/io.github.azizstark.TalisTray-symbolic.svg';
    assert.equal(readFileSync(join(destination,trayIcon),'utf8'),
        'share/icons/hicolor/scalable/status/io.github.azizstark.TalisTray-symbolic.svg');
    assert(spec.split('\n%files\n')[1].includes('%{_datadir}/icons/hicolor/scalable/status/io.github.azizstark.TalisTray-symbolic.svg'));
    console.log('PASS: RPM layout, both backends, system companion/library, checksum guard, automatic dependencies and no install-time user mutations');
} finally {
    rmSync(temporary, {recursive:true, force:true});
}
