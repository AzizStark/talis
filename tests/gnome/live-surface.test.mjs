// Logic-only lifecycle checks. These do not emulate Mutter/Wayland rendering.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import vm from 'node:vm';

const directory = new URL('../../integrations/gnome/', import.meta.url);
const source = readFileSync(new URL('live-surface.js', directory), 'utf8')
    .replace(/^import .*;\n/gm, '').replace('export class LiveSurface', 'class LiveSurface');
const inputSource = readFileSync(new URL('scene-input.js', directory), 'utf8')
    .replace(/^import .*;\n/gm, '').replace('export class SceneInput', 'class SceneInput');
const visibilitySource = readFileSync(new URL('scene-visibility.js', directory), 'utf8')
    .replace(/^import .*;\n/gm, '').replace('export class SceneVisibility', 'class SceneVisibility');
// Match GNOME 48's upstream MetaCursor enum, not invented compatibility names.
const GNOME48_CURSOR = Object.freeze({INVALID: 0, NONE: 1, DEFAULT: 2, POINTER: 5});
function fixture({spawnFails = false, sceneConfig = null, cursorEnum = GNOME48_CURSOR,
    fileTest = () => true} = {}) {
    let id = 0;
    const signals = new Map();
    const timeouts = new Map();
    const overlays = [];
    const stats = {spawn: 0, terminate: 0, close: 0, reads: 0, cancel: 0, conceal: [], writes: [], cursors: []};
    class Actor {
        constructor(props = {}) { Object.assign(this, props); this.children = []; this.destroyed = false; }
        add_child(child) { this.children.push(child); }
        insert_child_at_index(child, index) { this.children.splice(index, 0, child); }
        destroy() {
            assert.equal(this.destroyed, false); this.destroyed = true;
            for (const child of this.children) child.destroy();
            for (const [key, value] of signals) if (value.object === this) signals.delete(key);
        }
        set_text(text) { this.text = text; }
        connect(name, callback) { const key = ++id; signals.set(key, {object: this, name, callback}); return key; }
        connect_after(name, callback) { return this.connect(name, callback); }
        disconnect(key) { assert(signals.delete(key)); }
        get_title() { return this.title ?? 'Talis · Scene'; }
        transform_stage_point(x, y) {
            if (this.badTransform)
                return [false, NaN, NaN];
            return [true, (x - (this.x ?? 0)) / (this.scale_x ?? 1),
                (y - (this.y ?? 0)) / (this.scale_y ?? 1)];
        }
        activate() { this.activated = (this.activated ?? 0) + 1; }
        can_minimize() { return !this.hidden && !this.blockMinimize; }
        minimize() {
            stats.conceal.push('minimize');
            // Mutter 48 recalculates this capability when hidden from window lists.
            if (!this.can_minimize())
                return;
            this.minimized = true;
            for (const signal of signals.values()) {
                if (signal.object === this && signal.name === 'notify::minimized')
                    signal.callback();
            }
        }
    }
    const wm = new Actor();
    const stage = new Actor();
    const display = new Actor();
    display.set_cursor = cursor => {
        assert(Number.isInteger(cursor) && [GNOME48_CURSOR.DEFAULT, GNOME48_CURSOR.POINTER].includes(cursor),
            'undefined/INVALID cursor would abort Mutter, not throw a recoverable JS error');
        stats.cursor = cursor;
        stats.cursors.push(cursor);
    };
    const inputPipe = {
        write_all_async(bytes, _priority, _cancel, callback) {
            stats.writes.push(JSON.parse(new TextDecoder().decode(bytes)));
            inputPipe.pending = callback;
        },
        write_all_finish() {},
    };
    const process = {
        get_stdout_pipe: () => ({}),
        get_stdin_pipe: () => inputPipe,
        wait_async: (_cancel, callback) => { process.wait = callback; },
        wait_finish: () => {},
        send_signal: signal => { assert.equal(signal, 15); stats.terminate++; },
    };
    let output;
    const Gio = {
        File: {new_for_path: () => ({
            query_exists: () => sceneConfig !== null,
            load_contents: () => [true, new TextEncoder().encode(JSON.stringify(sceneConfig))],
        })},
        SubprocessFlags: {STDOUT_PIPE: 1, STDERR_MERGE: 2, STDIN_PIPE: 4},
        SubprocessLauncher: class {
            setenv() {} unsetenv() {} close() { stats.close++; }
        },
        Cancellable: class { cancel() { stats.cancel++; } },
        DataInputStream: class {
            constructor() { output = this; }
            read_line_async(_priority, _cancel, callback) { stats.reads++; this.pending = callback; }
            read_line_finish_utf8(result) { return [result]; }
        },
    };
    const client = {
        owns_window: window => window.owned === true,
        hide_from_window_list: window => { stats.conceal.push('hide'); window.hidden = true; },
        show_in_window_list: window => { stats.conceal.push('show'); window.hidden = false; },
        spawnv: (_display, argv) => {
            stats.argv = argv;
            stats.spawn++; if (spawnFails) throw new Error('spawn failed'); return process;
        },
    };
    const shield = new Actor({active: false, locked: false});
    const overview = new Actor({visible: false});
    const workspace = {index: () => 0};
    const workspaceManager = new Actor({get_active_workspace: () => workspace});
    const Main = {screenShield: shield, overview};
    const LiveSurface = vm.runInNewContext(`${inputSource}\n${visibilitySource}\n${source}\nLiveSurface;`, {
        Clutter: {Actor, EVENT_PROPAGATE: false, EVENT_STOP: true,
            EventType: {BUTTON_RELEASE: 'release', BUTTON_PRESS: 'press', MOTION: 'motion'},
            ScrollDirection: {UP: 'up', DOWN: 'down', SMOOTH: 'smooth'},
            Clone: class extends Actor { constructor(props) { super(props); overlays.push(this); } }},
        St: {Widget: Actor, Label: class extends Actor { constructor(props) { super(props); overlays.push(this); } }},
        Gio, Main, Cogl: {TextureComponents: {RGB: 2, RGBA: 3}},
        Meta: {WaylandClient: {new: () => client}, Cursor: cursorEnum, WindowClientType: {WAYLAND: 0, X11: 1}},
        GLib: {
            get_monotonic_time: () => 0,
            PRIORITY_DEFAULT: 0, SOURCE_REMOVE: false, SOURCE_CONTINUE: true, FileTest: {IS_EXECUTABLE: 1},
            build_filenamev: parts => parts.join('/'), file_test: fileTest,
            timeout_add_seconds: (_priority, _seconds, callback) => {
                const key = ++id; timeouts.set(key, callback); return key;
            },
            timeout_add: (_priority, _ms, callback) => {
                const key = ++id; timeouts.set(key, callback); return key;
            },
            source_remove: key => { assert(timeouts.delete(key)); },
        },
        global: {context: {}, display, stage, window_manager: wm, workspace_manager: workspaceManager,
            get_window_actors: () => [], get_current_time: () => 42},
        TextDecoder, TextEncoder,
        console: {log() {}, warn() {}, error() {}},
    });
    return {LiveSurface, Actor, signals, timeouts, overlays, stats, process, stage, inputPipe,
        shield, overview, workspaceManager, get output() { return output; }};
}

// Source checkout paths follow the flattened layout. Packaged extension paths
// stay relative to the UUID installation directory, independent of the repo.
const sceneConfig = JSON.parse(readFileSync(new URL('scene.json', directory), 'utf8'));
const sourceLayout = fixture({sceneConfig, fileTest: () => false});
const extensionPath = fileURLToPath(directory);
assert.equal(resolve(sourceLayout.LiveSurface.sceneSource(extensionPath).assets),
    fileURLToPath(new URL('../../assets', directory)));
assert.equal(resolve(sourceLayout.LiveSurface.binary(extensionPath)),
    fileURLToPath(new URL('../../target/release/talis-gnome-scene', directory)));
const metadata = JSON.parse(readFileSync(new URL('metadata.json', directory), 'utf8'));
assert.equal(metadata.uuid, 'talis@azizstark', 'source reorganization must not change installed identity');
const installedPath = `/example/gnome-shell/extensions/${metadata.uuid}`;
const installedLayout = fixture({sceneConfig});
assert.equal(installedLayout.LiveSurface.sceneSource(installedPath).assets, `${installedPath}/assets`);
assert.equal(installedLayout.LiveSurface.binary(installedPath), `${installedPath}/talis-gnome-scene`);

const f = fixture();
const live = new f.LiveSurface('/probe');
live.start([new f.Actor({width: 1920, height: 1080})]);
assert.equal(f.stats.spawn, 1);
assert.equal(f.stats.close, 1);
const unrelated = new f.Actor({meta_window: new f.Actor({owned: false})});
live._attach(unrelated);
assert.equal(unrelated.meta_window.hidden, undefined);
const owned = new f.Actor({meta_window: new f.Actor({owned: true})});
live._attach(owned);
assert.equal(owned.meta_window.hidden, true);
assert.equal(owned.meta_window.minimized, true);
assert.deepEqual(f.stats.conceal, ['show', 'minimize', 'hide']);
assert.equal(live._overlays[0].clone.source, owned);
assert.equal(live._overlays[0].clone.x, 1480);
assert.equal(live._overlays[0].clone.y, 798);
// External unminimize must restore the capability before re-minimizing.
owned.meta_window.minimized = false;
const minimizedHandler = [...f.signals.values()].find(signal => signal.name === 'notify::minimized');
minimizedHandler.callback();
assert.equal(owned.meta_window.minimized, true);
assert.equal(owned.meta_window.hidden, true);
assert.deepEqual(f.stats.conceal, ['show', 'minimize', 'hide', 'show', 'minimize', 'hide']);
const shownHandler = [...f.signals.values()].find(signal => signal.name === 'shown');
shownHandler.callback();
assert.equal(f.stats.terminate, 0);
f.output.pending(f.output, 'FRAME 60');
assert.equal(live._frames, 60);
const previous = [...live._overlays];
live.setScenes([new f.Actor({width: 1280, height: 720})]);
assert(previous.every(({clone}) => !clone || clone.destroyed));
assert.equal(live._overlays.length, 1);
assert.equal(live._overlays[0].clone.x, 840);
assert.equal(live._overlays[0].clone.y, 438);
f.output.pending(f.output, null); // EOF does not schedule another read.
assert.equal(f.stats.reads, 2);
live.stop();
live.stop();
assert.equal(f.stats.terminate, 1);
assert.equal(f.stats.cancel, 1);
assert.equal(f.signals.size, 0);
assert.equal(f.timeouts.size, 0);
assert(f.overlays.every(actor => actor.destroyed));

const timed = fixture();
const noSurface = new timed.LiveSurface('/probe');
noSurface.start([]);
const [timeoutId, timeout] = [...timed.timeouts][0];
timed.timeouts.delete(timeoutId); // GLib removes one-shot after callback.
timeout();
assert.equal(timed.stats.terminate, 1);
assert.equal(timed.signals.size, 0);

const failed = fixture({spawnFails: true});
const unavailable = new failed.LiveSurface('/probe');
assert.throws(() => unavailable.start([]), /spawn failed/);
unavailable.stop();
assert.equal(failed.stats.close, 1);
assert.equal(failed.signals.size, 0);
assert.equal(failed.stats.terminate, 0);

const exited = fixture();
const earlyExit = new exited.LiveSurface('/probe');
earlyExit.start([new exited.Actor({width: 1280, height: 720})]);
exited.process.wait(exited.process, {});
assert.equal(exited.stats.terminate, 1);
assert.equal(exited.signals.size, 0);
assert(exited.overlays.every(actor => actor.destroyed));

const destroyed = fixture();
const surfaceLoss = new destroyed.LiveSurface('/probe');
surfaceLoss.start([]);
const actor = new destroyed.Actor({meta_window: new destroyed.Actor({owned: true})});
surfaceLoss._attach(actor);
const destroyHandler = [...destroyed.signals.values()].find(signal => signal.name === 'destroy');
destroyHandler.callback();
assert.equal(destroyed.stats.terminate, 1);
assert.equal(destroyed.signals.size, 0);

// A refused minimize must not leave an ordinary app window plus a desktop clone.
for (const silentFailure of [false, true]) {
    const refused = fixture();
    const badSurface = new refused.LiveSurface('/probe');
    badSurface.start([new refused.Actor({width: 1280, height: 720})]);
    const window = new refused.Actor({owned: true, blockMinimize: !silentFailure});
    if (silentFailure)
        window.minimize = () => {}; // Capability was true, but request had no effect.
    badSurface._attach(new refused.Actor({meta_window: window}));
    assert.equal(refused.stats.terminate, 1);
    assert.equal(refused.signals.size, 0);
    assert.equal(refused.timeouts.size, 0);
    assert.equal(badSurface._overlays.length, 0);
}
console.log('PASS: concealment order/reentrancy/recovery/failure, owned-window isolation, placement/rebuild, telemetry/EOF, timeout, spawn failure, source/child exit and idempotent teardown');

const full = fixture({sceneConfig: {version: 3, scene: 'day', mode: 'music'}});
const real = new full.LiveSurface('/probe');
const desktop = new full.Actor({width: 1920, height: 1080});
const fallback = new full.Actor();
const button = new full.Actor();
desktop.add_child(fallback);
desktop.add_child(button);
real.start([desktop]);
assert.deepEqual(Array.from(full.stats.argv), ['/probe/talis-gnome-scene',
    '--asset-root', '/probe/assets', '--scene', 'day', '--mode', 'music', '--size', '1920x1080']);
// An owned Settings window must never be concealed or used as the scene clone.
real.openSettings();
const settings = new full.Actor({owned: true, title: 'Talis · Settings'});
real._attach(new full.Actor({meta_window: settings}));
assert.equal(settings.minimized, undefined);
assert.equal(settings.hidden, undefined);
assert.equal(settings.activated, 1);
assert.equal(real._source, undefined);
full.inputPipe.pending(full.inputPipe, {});
real._attach(new full.Actor({meta_window: new full.Actor({owned: true})}));
assert.equal(real._overlays[0].clone, null, 'static fallback stays until a successful rendered frame');
full.output.pending(full.output, 'FRAME 1');
const clone = real._overlays[0].clone;
assert.equal(clone.x, 0);
assert.equal(clone.y, 0);
assert.equal(clone.width, 1920);
assert.equal(clone.height, 1080);
assert.equal(desktop.children[1], clone);
assert(desktop.children.indexOf(button) > desktop.children.indexOf(clone));
assert.equal(real._frames, 1);
const inputLayout = {version: 1, generation: 1, layout: {width: 1920, height: 1080,
    hotspots: [{id: 'play_pause', rect: [600, 600, 20, 12]}, {id: 'volume', rect: [900, 600, 20, 30]}]}};
full.output.pending(full.output, `INPUT ${JSON.stringify(inputLayout)}`);
assert.equal(real._input._actors.length, 2);
const spans = real._input._actors[0].children;
assert(spans.length > 0 && spans.every(a => a.reactive && !a.can_focus));
assert(spans[0].width < 20, 'ellipse corners must stay click-through');
const press = [...full.signals.values()].find(s => s.object === spans[5] && s.name === 'button-press-event');
const event = (type, x, y, direction = 'down') => ({type: () => type, get_button: () => 1,
    get_coords: () => [x, y], get_scroll_direction: () => direction, get_scroll_delta: () => [0, 0.25]});
press.callback(spans[5], event('press', 610, 606));
assert.equal(full.stats.writes.at(-1).event, 'press');
full.inputPipe.pending(full.inputPipe, {});
const capture = [...full.signals.values()].find(s => s.object === full.stage && s.name === 'captured-event');
capture.callback(full.stage, event('release', 610, 606));
assert.equal(full.stats.writes.at(-1).event, 'release');
full.inputPipe.pending(full.inputPipe, {});
const knob = real._input._actors[1].children[10];
const scroll = [...full.signals.values()].find(s => s.object === knob && s.name === 'scroll-event');
scroll.callback(knob, event('scroll', 910, 610, 'smooth'));
assert.equal(full.stats.writes.at(-1).dy, 0.25);
full.inputPipe.pending(full.inputPipe, {});
// A release swallowed by a client window must not leave a permanently held button.
press.callback(spans[5], event('press', 610, 606));
full.inputPipe.pending(full.inputPipe, {});
const lostId = real._input._captureTimeout;
const lostRelease = full.timeouts.get(lostId);
full.timeouts.delete(lostId);
lostRelease();
assert.equal(full.stats.writes.at(-1).event, 'cancel');
assert.equal(real._input._capture, null);
full.inputPipe.pending(full.inputPipe, {});
press.callback(spans[5], event('press', 610, 606));
full.inputPipe.pending(full.inputPipe, {});
const focusChanged = [...full.signals.values()].find(s => s.name === 'notify::focus-window');
focusChanged.callback();
assert.equal(full.stats.writes.at(-1).event, 'cancel');
assert.equal(real._input._capture, null);
full.inputPipe.pending(full.inputPipe, {});
// Hiding Settings can destroy its compositor window; only a new owned map may
// become the replacement, and it must stay visible as an ordinary app window.
const settingsGone = [...full.signals.values()].find(s => s.object === settings && s.name === 'unmanaged');
settingsGone.callback();
real.openSettings();
full.inputPipe.pending(full.inputPipe, {});
const reopenedSettings = new full.Actor({owned: true, title: 'Talis · Settings'});
real._attach(new full.Actor({meta_window: reopenedSettings}));
assert.equal(reopenedSettings.activated, 1);
assert.equal(reopenedSettings.hidden, undefined);
// A scene rebuild cancels a held gesture. Clone scaling must not disable input.
press.callback(spans[5], event('press', 610, 606));
full.inputPipe.pending(full.inputPipe, {});
real.setScenes([new full.Actor({width: 1280, height: 720})]);
assert.equal(full.stats.writes.at(-1).event, 'cancel');
assert.equal(real._input._actors.length, 2);
full.inputPipe.pending(full.inputPipe, {});
// Reproduce the guest's work-area-constrained source and exercise the *same*
// viewport-to-clone mapping for pick spans and inverse command coordinates.
const offsetDesktop = new full.Actor({x: 100, y: 70, width: 2048, height: 1152,
    scale_x: 1.25, scale_y: 1.25});
real.setScenes([offsetDesktop]);
const constrained = {version: 1, generation: 2, layout: {width: 2048, height: 1120,
    viewport: [28, 0, 1991, 1120], hotspots: [
        {id: 'play_pause', rect: [740.14, 715.83, 39.32, 15.02]},
        {id: 'volume', rect: [1266.82, 691.95, 38.51, 36.19]}]}};
full.output.pending(full.output, `INPUT ${JSON.stringify(constrained)}`);
const shown = real._overlays[0].clone;
const scaleX = shown.width / constrained.layout.width;
const scaleY = shown.height / constrained.layout.height;
assert.equal(scaleX, scaleY, 'presentation must not stretch the scene');
assert(shown.x + 28 * scaleX <= 1e-8, 'left letterbox must be outside the scene');
assert(shown.x + (28 + 1991) * scaleX >= 2048 - 1e-8);
assert(shown.y <= 1e-8 && shown.y + 1120 * scaleY >= 1152 - 1e-8);
const scaledSpans = real._input._actors[0].children;
const rect = constrained.layout.hotspots[0].rect;
const sourcePoint = [rect[0] + rect[2] / 2, rect[1] + rect[3] / 2];
const localPoint = [shown.x + sourcePoint[0] * scaleX, shown.y + sourcePoint[1] * scaleY];
assert(scaledSpans.some(s => localPoint[0] >= s.x && localPoint[0] < s.x + s.width
    && localPoint[1] >= s.y && localPoint[1] < s.y + s.height), 'scaled center must be pickable');
const stagePoint = p => [offsetDesktop.x + p[0] * 1.25, offsetDesktop.y + p[1] * 1.25];
const scaledPress = [...full.signals.values()].find(s => s.object === scaledSpans[5] && s.name === 'button-press-event');
const hover = [...full.signals.values()].find(s => s.object === scaledSpans[5] && s.name === 'enter-event');
hover.callback();
assert.equal(full.stats.cursor, GNOME48_CURSOR.POINTER, 'enabled scaled hotspot advertises clickability');
const leave = [...full.signals.values()].find(s => s.object === scaledSpans[5] && s.name === 'leave-event');
leave.callback();
assert.equal(full.stats.cursor, GNOME48_CURSOR.DEFAULT);
hover.callback();
scaledPress.callback(scaledSpans[5], event('press', ...stagePoint(localPoint)));
const assertPoint = (kind, point) => {
    const sent = full.stats.writes.at(-1);
    assert.equal(sent.event, kind);
    assert.equal(sent.generation, 2);
    assert(Math.abs(sent.x - point[0]) < 1e-8 && Math.abs(sent.y - point[1]) < 1e-8);
};
assertPoint('press', sourcePoint);
full.inputPipe.pending(full.inputPipe, {});
capture.callback(full.stage, event('motion', ...stagePoint(localPoint)));
assertPoint('motion', sourcePoint);
full.inputPipe.pending(full.inputPipe, {});
capture.callback(full.stage, event('release', ...stagePoint(localPoint)));
assertPoint('release', sourcePoint);
full.inputPipe.pending(full.inputPipe, {});
const scaledKnob = real._input._actors[1].children[10];
const scaledScroll = [...full.signals.values()].find(s => s.object === scaledKnob && s.name === 'scroll-event');
const volumeRect = constrained.layout.hotspots[1].rect;
const volumePoint = [volumeRect[0] + volumeRect[2] / 2, volumeRect[1] + volumeRect[3] / 2];
scaledScroll.callback(scaledKnob, event('scroll', ...stagePoint([
    shown.x + volumePoint[0] * scaleX, shown.y + volumePoint[1] * scaleY]), 'smooth'));
assertPoint('scroll', volumePoint);
full.inputPipe.pending(full.inputPipe, {});
// Failed Clutter transforms cannot inject null coordinates into Rust's protocol.
offsetDesktop.badTransform = true;
const writesBefore = full.stats.writes.length;
scaledPress.callback(scaledSpans[5], event('press', 1, 1));
scaledScroll.callback(scaledKnob, event('scroll', 1, 1));
assert.equal(full.stats.writes.length, writesBefore);
offsetDesktop.badTransform = false;
// Empty allocations fail closed. A source viewport outside the surface is invalid.
real.setScenes([new full.Actor({width: 0, height: 720})]);
assert.equal(real._input._actors.length, 0);
assert.equal(full.stats.cursor, GNOME48_CURSOR.DEFAULT, 'rebuild resets a valid hover cursor');
assert.throws(() => real._input.constructor.validate({...constrained,
    layout: {...constrained.layout, viewport: [0, 0, 3000, 1120]}}), /viewport/);
console.log('PASS: constrained source viewport, bar-free aspect-preserving clone, scaled/offset picking and inverse press/release/motion/scroll, invalid transforms');
// Serialized async writes: never overlap, motion is coalesced, backlog bounded.
real._sendInput({event: 'motion', generation: 1, x: 1, y: 1});
for (let i = 0; i < 100; i++) real._sendInput({event: 'motion', generation: 1, x: i, y: 1});
assert.equal(real._writes.length, 1);
for (let i = 0; i < 100; i++) real._sendInput({event: 'scroll', generation: 1, x: 1, y: 1, dy: 1});
assert(real._writes.length <= 32);
real.stop();
assert.equal(full.signals.size, 0);
for (const config of [{version: 9, scene: 'day', mode: 'music'},
    {version: 3, scene: '../night', mode: 'music'}, {version: 3, scene: 'day', mode: 'command'}]) {
    const bad = fixture({sceneConfig: config});
    assert.throws(() => new bad.LiveSurface('/probe'), /Unsupported/);
}
const noFrames = fixture({sceneConfig: {version: 3, scene: 'night', mode: 'demo'}});
const stalled = new noFrames.LiveSurface('/probe');
stalled.start([new noFrames.Actor({width: 1280, height: 720})]);
stalled._attach(new noFrames.Actor({meta_window: new noFrames.Actor({owned: true})}));
const stalledTimeout = stalled._startupTimeout;
const stall = noFrames.timeouts.get(stalledTimeout);
noFrames.timeouts.delete(stalledTimeout);
stall();
assert.equal(noFrames.stats.terminate, 1);
console.log('PASS: real-scene placement/fallback, Settings isolation, elliptical input, button/wheel forwarding, capture cancellation, bounded asynchronous queue and teardown');

const logoFixture = fixture({sceneConfig: {version: 3, scene: 'day', mode: 'music'}});
const logoSurface = new logoFixture.LiveSurface('/probe');
logoSurface.start([new logoFixture.Actor({width: 1920, height: 1080})]);
logoSurface._attach(new logoFixture.Actor({meta_window: new logoFixture.Actor({owned: true})}));
logoFixture.output.pending(logoFixture.output, 'FRAME 1');
logoFixture.output.pending(logoFixture.output, `INPUT ${JSON.stringify({version: 1, generation: 1,
    layout: {width: 1920, height: 1080, hotspots: [{id: 'settings', rect: [660, 720, 60, 40]}]}})}`);
const logoActor = logoSurface._input._actors[0].children[20];
const logoPress = [...logoFixture.signals.values()].find(s => s.object === logoActor && s.name === 'button-press-event');
const logoRelease = [...logoFixture.signals.values()].find(s => s.object === logoFixture.stage && s.name === 'captured-event');
logoPress.callback(logoActor, event('press', 690, 740));
logoFixture.inputPipe.pending(logoFixture.inputPipe, {});
logoRelease.callback(logoFixture.stage, event('motion', 1, 1));
logoFixture.inputPipe.pending(logoFixture.inputPipe, {});
logoRelease.callback(logoFixture.stage, event('release', 690, 740));
assert(!logoFixture.stats.writes.some(w => w.event === 'settings'), 'Dragged-out logo click opened Settings');
logoPress.callback(logoActor, event('press', 690, 740));
logoFixture.inputPipe.pending(logoFixture.inputPipe, {});
logoRelease.callback(logoFixture.stage, event('release', 690, 740));
logoFixture.inputPipe.pending(logoFixture.inputPipe, {});
assert.equal(logoFixture.stats.writes.at(-1).event, 'settings');
assert(logoSurface._settingsRequested, 'Logo bypassed owned Settings activation');
logoSurface.stop();
assert.equal(logoFixture.signals.size, 0);
console.log('PASS: turntable logo Settings activation without player controls, drag-out cancellation and cleanup');

const malformed = fixture({sceneConfig: {version: 3, scene: 'day', mode: 'music'}});
const badLayout = new malformed.LiveSurface('/probe');
badLayout.start([new malformed.Actor({width: 1280, height: 720})]);
malformed.output.pending(malformed.output, 'INPUT {"version":1,"generation":1,"layout":{"width":1280,"height":720,"hotspots":[{"id":"volume","rect":[-1,0,20,20]}]}}');
assert.equal(malformed.stats.terminate, 1);
assert.equal(malformed.signals.size, 0);
assert.equal(malformed.timeouts.size, 0);
console.log('PASS: malformed Shell input layout fails closed with lifecycle cleanup');

// ABI regressions: absent names and invalid numeric values must never cross
// into the compositor, even during leave/rebuild/disable cleanup. Click IPC
// remains available without a hand cursor when an enum cannot be resolved.
for (const cursorEnum of [GNOME48_CURSOR,
    {INVALID: 0, NONE: 1, DEFAULT: 2}, {},
    {INVALID: 0, NONE: 1, DEFAULT: 0, POINTER: 0},
    {INVALID: 0, NONE: 1, DEFAULT: 2, POINTER: 1},
    {DEFAULT: 2, POINTER: 1},
    {INVALID: 0, NONE: 1, DEFAULT: undefined, POINTER: NaN},
    {INVALID: 0, NONE: 1, DEFAULT: -1, POINTER: 5.5}]) {
    const cursorFixture = fixture({sceneConfig: {version: 3, scene: 'day', mode: 'music'}, cursorEnum});
    const cursorLive = new cursorFixture.LiveSurface('/probe');
    const cursorScene = new cursorFixture.Actor({width: 1920, height: 1080});
    cursorLive.start([cursorScene]);
    cursorLive._attach(new cursorFixture.Actor({meta_window: new cursorFixture.Actor({owned: true})}));
    cursorFixture.output.pending(cursorFixture.output, 'FRAME 1');
    cursorFixture.output.pending(cursorFixture.output, `INPUT ${JSON.stringify(inputLayout)}`);
    const widget = cursorLive._input._actors[0].children[5];
    const callback = name => [...cursorFixture.signals.values()].find(s => s.object === widget && s.name === name).callback;
    callback('enter-event')();
    callback('leave-event')();
    callback('enter-event')();
    callback('button-press-event')(widget, event('press', 610, 606));
    assert.equal(cursorFixture.stats.writes.at(-1).event, 'press');
    cursorFixture.inputPipe.pending(cursorFixture.inputPipe, {});
    const stageCapture = [...cursorFixture.signals.values()].find(s => s.object === cursorFixture.stage && s.name === 'captured-event');
    stageCapture.callback(cursorFixture.stage, event('release', 610, 606));
    assert.equal(cursorFixture.stats.writes.at(-1).event, 'release');
    cursorFixture.inputPipe.pending(cursorFixture.inputPipe, {});
    cursorLive.setScenes([cursorScene]);
    cursorLive.stop();
    assert.equal(cursorFixture.signals.size, 0);
    assert.equal(cursorFixture.timeouts.size, 0);
    if (cursorEnum === GNOME48_CURSOR)
        assert.deepEqual(cursorFixture.stats.cursors, [5, 2, 5, 2]);
    else if (!Number.isInteger(cursorEnum.POINTER) || cursorEnum.POINTER <= 1)
        assert(!cursorFixture.stats.cursors.includes(GNOME48_CURSOR.POINTER));
}
console.log('PASS: GNOME 48 cursor enum, hover/leave/rebuild/disable, missing/INVALID/NONE/nonfinite enum refusal; click delivery independent of cursor');
