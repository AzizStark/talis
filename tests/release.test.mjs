// Exercise the real assembly command with tiny ELF fixtures, not a Rust build.
import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,writeFileSync,readFileSync,copyFileSync,existsSync,readdirSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';

const temporary=mkdtempSync(join(tmpdir(),'talis-release-'));
try {
    const repo=join(temporary,"source with 'quote"),commands=join(temporary,'commands');
    const libraryDir=join(temporary,'layer library'),target=join(temporary,'cargo target');
    for(const name of ['scripts','packaging','src','assets/ui/branding','integrations/gnome'])
        mkdirSync(join(repo,name),{recursive:true});
    for(const directory of [commands,libraryDir]) mkdirSync(directory);
    for(const name of ['release.sh','build.sh','check-deps.sh','bundle.sh','stage-release.sh','launch.sh','session.sh'])
        copyFileSync(new URL(`../scripts/${name}`,import.meta.url),join(repo,'scripts',name));
    for(const name of ['setup-gnome.sh','PORTABLE.md'])
        copyFileSync(new URL(`../packaging/${name}`,import.meta.url),join(repo,'packaging',name));
    writeFileSync(join(repo,'Cargo.toml'),'[package]\nname = "talis"\nversion = "0.1.0"\n');
    for(const name of ['Cargo.lock','LICENSE','LICENSING.md','src/main.rs',
        'packaging/io.github.azizstark.Talis.desktop','packaging/io.github.azizstark.Talis.metainfo.xml',
        'assets/ui/branding/io.github.azizstark.Talis.svg','integrations/gnome/extension.js','integrations/gnome/metadata.json'])
        writeFileSync(join(repo,name),name);
    const call=(program,args,env={})=>spawnSync(program,args,{cwd:repo,encoding:'utf8',timeout:15000,env:{...process.env,...env}});
    const ok=r=>assert.equal(r.status,0,r.stderr||r.stdout);
    const library=join(libraryDir,'libgtk4-layer-shell.so.0'),wayland=join(temporary,'wayland');
    const license=join(temporary,'layer-LICENSE');
    writeFileSync(license,'upstream layer-shell notice');
    writeFileSync(join(temporary,'fixture.c'),'int main(void) { return 0; }\n');
    ok(call('cc',['-shared','-fPIC','-Wl,-soname,libgtk4-layer-shell.so.0',join(temporary,'fixture.c'),'-o',library]));
    ok(call('cc',[join(temporary,'fixture.c'),'-Wl,--no-as-needed',library,'-o',wayland]));
    writeFileSync(join(commands,'rustc'),'#!/bin/sh\nexit 0\n',{mode:0o755});
    writeFileSync(join(commands,'pkg-config'),`#!/bin/sh
case "$1" in
  --variable=libdir) printf '%s\\n' "$TEST_LAYER_DIRECTORY" ;;
  --modversion) echo 4.14.5 ;;
esac
`,{mode:0o755});
    writeFileSync(join(commands,'cargo'),`#!/usr/bin/env bash
set -euo pipefail
printf '%s|%s\\n' "$CARGO_TARGET_DIR" "$*" >> "$TEST_BUILD_TRACE"
mkdir -p "$CARGO_TARGET_DIR/release"
if [[ " $* " == *' --features wayland-desktop '* ]]; then
  [[ \${TEST_FAIL_WAYLAND:-0} != 1 ]] || exit 19
  cp "$TEST_WAYLAND_BINARY" "$CARGO_TARGET_DIR/release/talis"
else
  cp /bin/true "$CARGO_TARGET_DIR/release/talis"
  cp /bin/true "$CARGO_TARGET_DIR/release/talis-gnome-scene"
  if [[ \${TEST_MUTATE_SOURCE:-0} == 1 ]]; then printf 'changed\\n' >> src/main.rs; fi
fi
if [[ -n \${TEST_OCCUPY_OUTPUT:-} ]]; then
  mkdir -p "$TEST_OCCUPY_OUTPUT"
  printf 'preserve me' > "$TEST_OCCUPY_OUTPUT/sentinel"
fi
`,{mode:0o755});
    const trace=join(temporary,'build-trace');
    const env={PATH:`${commands}:/usr/bin:/bin`,CARGO_TARGET_DIR:target,CARGO_BUILD_TARGET:'',
        TEST_LAYER_DIRECTORY:libraryDir,TEST_BUILD_TRACE:trace,TEST_WAYLAND_BINARY:wayland};
    const release=(destination,changes={},notice=license)=>call('bash',[join(repo,'scripts/release.sh'),destination,notice],{...env,...changes});
    assert.notEqual(release('relative').status,0);
    assert.notEqual(release('/').status,0);
    assert.notEqual(release(join(temporary,'missing-license'),{},'/missing/license').status,0);
    assert(!existsSync(trace),'invalid inputs must not build');
    const failed=join(temporary,'failed');
    assert.equal(release(failed,{TEST_FAIL_WAYLAND:'1'}).status,19);
    assert(!existsSync(failed),'a failed variant must not leave publishable output');
    const changed=join(temporary,'changed');
    const mutation=release(changed,{TEST_MUTATE_SOURCE:'1'});
    assert.notEqual(mutation.status,0);
    assert(mutation.stderr.includes('inputs changed'));
    assert(!existsSync(changed));
    const raced=join(temporary,'raced');
    assert.notEqual(release(raced,{TEST_OCCUPY_OUTPUT:raced}).status,0);
    assert.equal(readFileSync(join(raced,'sentinel'),'utf8'),'preserve me');
    const destination=join(temporary,"release with 'quote");
    ok(release(destination));
    assert.deepEqual(readdirSync(destination).sort(),['SHA256SUMS','talis-0.1.0-portable-linux-x86_64.tar.gz','talis-gnome-0.1.0.shell-extension.zip']);
    ok(spawnSync('sha256sum',['-c','SHA256SUMS'],{cwd:destination,encoding:'utf8'}));
    const archive=call('tar',['-tzf',join(destination,'talis-0.1.0-portable-linux-x86_64.tar.gz')]); ok(archive);
    for(const name of ['setup-gnome.sh','usr/lib/talis/session.sh','usr/lib/talis/talis-wayland-bin',
        'usr/lib/talis/talis-gnome-scene','usr/share/talis/gnome/metadata.json'])
        assert(archive.stdout.includes(`talis-linux-x86_64/${name}\n`));
    const zip=call('unzip',['-Z1',join(destination,'talis-gnome-0.1.0.shell-extension.zip')]); ok(zip);
    assert(zip.stdout.includes('metadata.json\n')&&zip.stdout.includes('LICENSE\n'));
    const builds=readFileSync(trace,'utf8');
    assert(builds.includes(`${target}/packages/x11|build --locked --release --bin talis --bin talis-gnome-scene`));
    assert(builds.includes(`${target}/packages/wayland|build --locked --release --features wayland-desktop --bin talis`));
    const before=readFileSync(join(destination,'SHA256SUMS'),'utf8');
    assert.notEqual(release(destination).status,0,'existing releases must never be replaced');
    assert.equal(readFileSync(join(destination,'SHA256SUMS'),'utf8'),before);
    assert(!readdirSync(temporary).some(n=>n.startsWith('.talis-release.')),'temporary payloads must be cleaned');
    console.log('PASS: variant isolation/freshness, build failure, changed-input rejection, output races, complete portable/GNOME artifacts, checksums, relocation and no-clobber publication');
} finally {rmSync(temporary,{recursive:true,force:true});}
