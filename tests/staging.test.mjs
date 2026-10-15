// Packaging layout only: no system installation, compiler, GTK or live player.
import assert from 'node:assert/strict';
import {mkdtempSync, mkdirSync, writeFileSync, readFileSync, copyFileSync, existsSync, readlinkSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';

const temporary = mkdtempSync(join(tmpdir(), 'talis-stage-'));
try {
    const repo = join(temporary, "repo with 'quote");
    const binaries = join(temporary, 'release');
    const destination = join(temporary, 'stage');
    for (const path of ['scripts','packaging','assets/ui/branding','blender','src'])
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
    const stage = dest => spawnSync('bash',[join(repo,'scripts/stage-release.sh'),dest,binaries],{encoding:'utf8',timeout:10000});
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
    console.log('PASS: runtime allowlist, relative symlinks, quoted paths and non-destructive staging guards');
} finally {
    rmSync(temporary,{recursive:true,force:true});
}
