// Build/launch logic only: no GTK session, package installation or real player.
import assert from 'node:assert/strict';
import {mkdtempSync, mkdirSync, readFileSync, writeFileSync, copyFileSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';

const temporary = mkdtempSync(join(tmpdir(), 'talis-launcher-'));
try {
    const root = join(temporary, "repo with space and 'quote");
    const commands = join(temporary, 'commands');
    mkdirSync(join(root,'scripts'), {recursive:true});
    mkdirSync(commands);
    for (const name of ['run.sh','build.sh','check-deps.sh'])
        copyFileSync(new URL(`../scripts/${name}`,import.meta.url),join(root,'scripts',name));
    const executable = '#!/usr/bin/env bash\nprintf "APP=%s\\n" "$PWD"\nprintf "%s\\n" "$@"\nprintf "BACKEND=%s\\n" "${GDK_BACKEND:-x11}"\n';
    writeFileSync(join(temporary,'fake-talis'), executable, {mode:0o755});
    writeFileSync(join(commands,'cargo'), `#!/usr/bin/env bash
set -e
[[ \${FORBID_BUILD:-0} != 1 ]] || exit 90
printf '%s\\n' "$*" >> "$BUILD_TRACE"
[[ \${CARGO_FAIL:-0} != 1 ]] || exit 17
mkdir -p "\${CARGO_TARGET_DIR:-target}/release"
cp "$FAKE_TALIS" "\${CARGO_TARGET_DIR:-target}/release/talis"
`, {mode:0o755});
    writeFileSync(join(commands,'rustc'), '#!/usr/bin/env bash\nexit 0\n', {mode:0o755});
    writeFileSync(join(commands,'pkg-config'), '#!/usr/bin/env bash\n[[ ${FORBID_BUILD:-0} != 1 ]] || exit 90\n[[ ${MISSING_GTK:-0} != 1 ]] || exit 1\nif [[ $1 == --modversion ]]; then echo 4.14.5; fi\n', {mode:0o755});
    const env = {...process.env, PATH:`${commands}:/usr/bin:/bin`,
        CARGO_TARGET_DIR:'target', BUILD_TRACE:join(temporary,'build.log'), FAKE_TALIS:join(temporary,'fake-talis'),
        XDG_SESSION_TYPE:'x11', XDG_CURRENT_DESKTOP:'X-Cinnamon', WAYLAND_DISPLAY:'',
        TALIS_BACKEND:'auto', TALIS_GL_BACKEND:'auto'};
    const run = (script,args=[],changes={}) => spawnSync('bash',[join(root,'scripts',script),...args], {
        cwd:temporary, env:{...env,...changes}, encoding:'utf8', timeout:10000,
    });
    const ok = result => assert.equal(result.status,0,result.stderr || result.stdout);
    let result = run('run.sh',['--scene','night']); ok(result);
    assert(result.stdout.includes(`APP=${root}`));
    assert(result.stdout.includes('--desktop\n--arm\n--arm-flow\n--now-playing\n--scene\nnight'));
    assert(!result.stdout.includes('--native-patch'), 'the binary owns normal desktop presentation policy');
    assert(readFileSync(env.BUILD_TRACE,'utf8').includes('build --locked --release --bin talis'));
    const before = readFileSync(env.BUILD_TRACE,'utf8');
    result = run('run.sh'); ok(result);
    assert.equal(readFileSync(env.BUILD_TRACE,'utf8'),before + before,
        'every source launch delegates freshness checks to Cargo, even with an existing binary');
    mkdirSync(join(root,'src'));
    writeFileSync(join(root,'src','main.rs'),'changed source');
    result = run('run.sh',[],{CARGO_FAIL:'1'});
    assert.equal(result.status,17,'propagate the build failure');
    assert(!result.stdout.includes('APP='),'never launch the old executable after a build failure');
    result = run('run.sh',[],{FORBID_BUILD:'1'});
    assert.notEqual(result.status,0,'source launches require build dependencies');
    assert(!result.stdout.includes('APP='));
    result = spawnSync(join(root,'target','release','talis'),['--desktop'], {
        cwd:root,env:{...env,FORBID_BUILD:'1'},encoding:'utf8',timeout:10000,
    }); ok(result);
    assert(result.stdout.includes('APP='),'direct binary launches need no build tools');
    assert.notEqual(run('run.sh',[],{TALIS_BACKEND:'invalid'}).status,0);
    assert.notEqual(run('build.sh',['x11'],{MISSING_GTK:'1'}).status,0);
    result = run('run.sh',[],{MISSING_GTK:'1'});
    assert.notEqual(result.status,0);
    assert(!result.stdout.includes('APP='),'dependency failure must not launch an old executable');
    assert.notEqual(run('run.sh',[],{XDG_SESSION_TYPE:'wayland',WAYLAND_DISPLAY:'wayland-test',XDG_CURRENT_DESKTOP:'GNOME'}).status,0);
    ok(run('build.sh',['wayland']));
    const library = join(temporary,'layer-shell.so.0'); writeFileSync(library,'fixture');
    result = run('run.sh',[],{XDG_SESSION_TYPE:'wayland',WAYLAND_DISPLAY:'wayland-test',
        XDG_CURRENT_DESKTOP:'Hyprland',TALIS_LAYER_SHELL_LIB:library}); ok(result);
    assert(result.stdout.includes('--experimental-wayland'));
    assert(result.stdout.includes('BACKEND=wayland'));
    assert(!result.stdout.includes('--native-patch'));
    assert(readFileSync(env.BUILD_TRACE,'utf8').trimEnd().endsWith('build --locked --release --features wayland-desktop --bin talis'));
    result = run('run.sh'); ok(result);
    assert(readFileSync(env.BUILD_TRACE,'utf8').trimEnd().endsWith('build --locked --release --bin talis'),
        'switching back to X11 must ask Cargo for the non-layer-shell build');
    const externalTarget = join(temporary,"target with 'quote");
    result = run('run.sh',['--scene','day'],{CARGO_TARGET_DIR:externalTarget}); ok(result);
    assert(result.stdout.includes('--scene\nday'));
    assert(readFileSync(join(externalTarget,'release','talis'),'utf8').includes('APP='));
    assert.notEqual(run('run.sh',[],{TALIS_BACKEND:'wayland',TALIS_LAYER_SHELL_LIB:'/missing/library'}).status,0);
    console.log('PASS: source freshness checks, failed-build safety, backend switching, quoting/relocation, external target directories, SDK-free direct launches, dependency errors, GNOME refusal and process-local Wayland runtime');
} finally {
    rmSync(temporary,{recursive:true,force:true});
}
