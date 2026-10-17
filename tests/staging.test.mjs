// Packaging layout only: no system installation, compiler, GTK or live player.
import assert from 'node:assert/strict';
import {mkdtempSync, mkdirSync, writeFileSync, readFileSync, copyFileSync, existsSync, readlinkSync, readdirSync, statSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';

const temporary = mkdtempSync(join(tmpdir(), 'talis-stage-'));
try {
    const repo = join(temporary, "repo with 'quote");
    const binaries = join(temporary, 'release');
    const destination = join(temporary, 'stage');
    for (const path of ['scripts','packaging','assets/ui/branding','blender','src','integrations/gnome'])
        mkdirSync(join(repo,path), {recursive:true});
    mkdirSync(binaries);
    for (const name of ['stage-release.sh','launch.sh','session.sh'])
        copyFileSync(new URL(`../scripts/${name}`,import.meta.url),join(repo,'scripts',name));
    for (const name of ['talis','talis-gnome-scene'])
        writeFileSync(join(binaries,name),'#!/bin/sh\nexit 0\n',{mode:0o755});
    for (const name of ['LICENSE','LICENSING.md','blender/not-runtime.blend','src/not-runtime.rs',
        'packaging/io.github.azizstark.Talis.desktop','packaging/io.github.azizstark.Talis.metainfo.xml',
        'assets/ui/branding/io.github.azizstark.Talis.svg'])
        writeFileSync(join(repo,name),name);
    const companion = new URL('../integrations/gnome/',import.meta.url);
    const companionFiles = readdirSync(companion).filter(name => /\.(js|json)$/.test(name));
    for (const name of companionFiles)
        copyFileSync(new URL(name,companion),join(repo,'integrations/gnome',name));
    writeFileSync(join(repo,'integrations/gnome/not-runtime.blend'),'not a runtime asset');
    const stage = (dest,...options) => spawnSync('bash',[join(repo,'scripts/stage-release.sh'),dest,binaries,...options],{encoding:'utf8',timeout:10000});
    const result = stage(destination);
    assert.equal(result.status,0,result.stderr);
    assert.equal(readlinkSync(join(destination,'usr/bin/talis')),'../lib/talis/launch.sh');
    assert.equal(readFileSync(join(destination,'usr/lib/talis/talis-bin'),'utf8'),'#!/bin/sh\nexit 0\n');
    assert(existsSync(join(destination,'usr/share/talis/assets/ui/branding/io.github.azizstark.Talis.svg')));
    for (const path of ['blender','src','scripts','usr/share/talis/blender','usr/share/talis/src'])
        assert(!existsSync(join(destination,path)),path+' leaked into runtime payload');
    assert.notEqual(stage(destination).status,0,'existing staging files must not be overwritten');
    assert.notEqual(stage('/').status,0,'system root must never be a staging target');
    assert.notEqual(stage('relative').status,0,'ambiguous relative staging paths are refused');
    assert.equal(readFileSync(join(destination,'usr/share/doc/talis/LICENSE'),'utf8'),'LICENSE');
    assert(!existsSync(join(destination,'usr/share/gnome-shell')), 'ordinary archives keep portable setup unchanged');
    const native = join(temporary,'native');
    assert.equal(stage(native,'--with-gnome-extension').status,0);
    const extension = join(native,'usr/share/gnome-shell/extensions/talis@azizstark');
    assert.deepEqual(readdirSync(extension).sort(),[...companionFiles,'LICENSE'].sort());
    for (const name of companionFiles) {
        assert.deepEqual(readFileSync(join(extension,name)),readFileSync(new URL(name,companion)));
        assert.equal(statSync(join(extension,name)).mode & 0o777,0o644);
    }
    assert.equal(statSync(extension).mode & 0o777,0o755);
    assert.equal(JSON.parse(readFileSync(join(extension,'metadata.json'),'utf8')).uuid,'talis@azizstark');
    assert(!existsSync(join(native,'home')) && !existsSync(join(native,'etc')),
        'staging must not write user files, autostart or system extension-enabling policy');
    const invalid = join(temporary,'invalid');
    assert.notEqual(stage(invalid,'--unknown').status,0);
    assert(!existsSync(invalid),'reject unknown options before writing');
    assert(readFileSync(new URL('../debian/build.sh',import.meta.url),'utf8').includes('--with-gnome-extension'),
        'Debian packaging must request the included companion');
    console.log('PASS: runtime allowlist, complete system companion, permissions, relative symlinks, quoted paths and non-destructive staging guards');
} finally {
    rmSync(temporary,{recursive:true,force:true});
}
