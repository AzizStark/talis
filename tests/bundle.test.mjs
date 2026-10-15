// Portable staging/setup only. No real desktop, extension or player is modified.
import assert from 'node:assert/strict';
import {mkdtempSync, mkdirSync, writeFileSync, readFileSync, copyFileSync, existsSync, readlinkSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';

const temporary = mkdtempSync(join(tmpdir(), 'talis-bundle-'));
try {
    const repo = join(temporary, "repo with 'quote");
    const binaries = join(temporary, 'release');
    const destination = join(temporary, "bundle with 'quote");
    for (const path of ['scripts','packaging','assets/ui/branding','integrations/gnome'])
        mkdirSync(join(repo,path), {recursive:true});
    mkdirSync(binaries);
    for (const name of ['stage-release.sh','launch.sh','session.sh','bundle.sh'])
        copyFileSync(new URL(`../scripts/${name}`,import.meta.url),join(repo,'scripts',name));
    for (const name of ['setup-gnome.sh','PORTABLE.md'])
        copyFileSync(new URL(`../packaging/${name}`,import.meta.url),join(repo,'packaging',name));
    for (const name of ['LICENSE','LICENSING.md','packaging/io.github.azizstark.Talis.desktop',
        'packaging/io.github.azizstark.Talis.metainfo.xml','assets/ui/branding/io.github.azizstark.Talis.svg',
        'integrations/gnome/extension.js','integrations/gnome/metadata.json'])
        writeFileSync(join(repo,name),name);
    for (const name of ['talis','talis-gnome-scene'])
        copyFileSync('/bin/true',join(binaries,name));
    const call = (command,args,env={}) => spawnSync(command,args, {
        encoding:'utf8',env:{...process.env,...env},timeout:10000,
    });
    const ok = result => assert.equal(result.status,0,result.stderr || result.stdout);
    const library = join(binaries,'libgtk4-layer-shell.so.0');
    writeFileSync(join(temporary,'fixture.c'),'int main(void) { return 0; }\n');
    ok(call('cc',['-shared','-fPIC','-Wl,-soname,libgtk4-layer-shell.so.0',join(temporary,'fixture.c'),'-o',library]));
    const wayland = join(binaries,'wayland');
    ok(call('cc',[join(temporary,'fixture.c'),'-Wl,--no-as-needed',library,'-o',wayland]));
    const license = join(temporary,'layer-LICENSE');
    writeFileSync(license,'layer-shell license fixture');
    const stage = (binary=wayland, lib=library, notice=license) => call('bash',[
        join(repo,'scripts/bundle.sh'),destination,binaries,binary,lib,notice,
    ]);
    assert.notEqual(stage(join(binaries,'talis')).status,0,'reject a non-Wayland binary');
    assert.notEqual(stage(wayland,library,'/missing/license').status,0,'license is mandatory');
    copyFileSync(wayland,join(binaries,'talis'));
    assert.notEqual(stage().status,0,'reject an accidentally feature-enabled X11 executable');
    copyFileSync('/bin/true',join(binaries,'talis'));
    assert(!existsSync(destination),'validate inputs before staging');
    ok(stage());
    assert.equal(readlinkSync(join(destination,'talis')),'usr/bin/talis');
    assert.equal(readFileSync(join(destination,'usr/share/doc/talis/gtk4-layer-shell-LICENSE'),'utf8'),'layer-shell license fixture');
    assert(existsSync(join(destination,'usr/lib/talis/talis-wayland-bin')));
    assert(existsSync(join(destination,'usr/lib/talis/session.sh')));
    assert(existsSync(join(destination,'usr/share/talis/gnome/extension.js')));
    assert(!existsSync(join(destination,'src')));
    assert.notEqual(stage().status,0,'never overwrite an occupied bundle');

    const commands = join(temporary,'commands'); mkdirSync(commands);
    writeFileSync(join(commands,'gnome-shell'),'#!/bin/sh\necho "GNOME Shell ${FAKE_GNOME_VERSION:-48.0}"\n',{mode:0o755});
    const data = join(temporary,"user data with 'quote");
    const setup = changes => call('bash',[join(destination,'setup-gnome.sh')], {
        PATH:`${commands}:/usr/bin:/bin`,XDG_DATA_HOME:data,...changes,
    });
    if (process.getuid() === 0) {
        assert.notEqual(setup().status,0,'setup refuses root');
    } else {
        assert.notEqual(setup({FAKE_GNOME_VERSION:'49.0'}).status,0,'no untested Shell versions');
        assert(!existsSync(data));
        assert.notEqual(setup({FAKE_GNOME_VERSION:'51.0'}).status,0);
        assert.notEqual(setup({FAKE_GNOME_VERSION:'50.alpha'}).status,0);
        ok(setup());
        const extension = join(data,'gnome-shell/extensions/talis@azizstark');
        assert.equal(readlinkSync(join(extension,'assets')),join(destination,'usr/share/talis/assets'));
        assert.equal(readlinkSync(join(extension,'talis-gnome-scene')),join(destination,'usr/bin/talis-gnome-scene'));
        const desktop = join(data,'applications/io.github.azizstark.Talis.desktop');
        const entry = readFileSync(desktop,'utf8');
        assert(entry.includes(`Exec=/usr/bin/env "${destination}/talis"`));
        assert(entry.includes(`Icon=${destination}/usr/share/talis/assets/ui/branding/io.github.azizstark.Talis.svg`));
        assert(!entry.includes('DBusActivatable=true'), 'activation must go through the companion, not a second GTK process');
        const before = readFileSync(join(extension,'extension.js'),'utf8');
        assert.notEqual(setup().status,0,'never replace an existing extension');
        assert.equal(readFileSync(join(extension,'extension.js'),'utf8'),before);
        const modernData = join(temporary,'gnome50 data');
        ok(setup({FAKE_GNOME_VERSION:'50.1',XDG_DATA_HOME:modernData}));
        assert(existsSync(join(modernData,'gnome-shell/extensions/talis@azizstark/extension.js')));
        const occupiedData = join(temporary,'occupied data');
        mkdirSync(join(occupiedData,'applications'), {recursive:true});
        const occupiedEntry = join(occupiedData,'applications/io.github.azizstark.Talis.desktop');
        writeFileSync(occupiedEntry,'user-owned entry');
        assert.notEqual(setup({XDG_DATA_HOME:occupiedData}).status,0);
        assert.equal(readFileSync(occupiedEntry,'utf8'),'user-owned entry');
        assert(!existsSync(join(occupiedData,'gnome-shell/extensions/talis@azizstark')));
        // Validate desktop-entry escaping with the real GLib parser when the
        // optional local PyGObject test dependency is available. No app launch.
        const gio = call('/usr/bin/python3',['-c','from gi.repository import Gio']);
        if (gio.status === 0) {
            const unusual = join(temporary, 'bundle " $ ` \\ % with\nnewline');
            mkdirSync(unusual);
            copyFileSync(join(destination,'setup-gnome.sh'),join(unusual,'setup-gnome.sh'));
            // Keep the actual launcher/assets as-is; only relocate the bundle.
            const {symlinkSync} = await import('node:fs');
            symlinkSync(join(destination,'usr'),join(unusual,'usr'));
            symlinkSync(join(destination,'talis'),join(unusual,'talis'));
            const unusualData = join(temporary,'unusual-data');
            ok(call('bash',[join(unusual,'setup-gnome.sh')],{
                PATH:`${commands}:/usr/bin:/bin`, XDG_DATA_HOME:unusualData,
            }));
            const parsed = call('/usr/bin/python3',['-c',
                'from gi.repository import Gio, GLib; import sys; app=Gio.DesktopAppInfo.new_from_filename(sys.argv[1]); assert app; assert app.get_executable()=="/usr/bin/env"; ok,args=GLib.shell_parse_argv(app.get_commandline()); assert args[1].replace("%%","%")==sys.argv[2], repr(args); assert app.get_icon().get_file().get_path()==sys.argv[3]',
                join(unusualData,'applications/io.github.azizstark.Talis.desktop'),
                join(unusual,'talis'), join(unusual,'usr/share/talis/assets/ui/branding/io.github.azizstark.Talis.svg'),
            ]);
            assert.equal(parsed.status,0,parsed.stderr + readFileSync(join(unusualData,'applications/io.github.azizstark.Talis.desktop'),'utf8'));
        }
    }
    console.log('PASS: dual-backend staging, ELF/SONAME guards, mandatory library notice, relocation, opt-in GNOME setup and no-clobber checks');
} finally {
    rmSync(temporary,{recursive:true,force:true});
}
