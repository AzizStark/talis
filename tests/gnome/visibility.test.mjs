// Verified GNOME 48 API-shaped fixtures, not actual compositor acceptance.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
const directory = new URL('../../integrations/gnome/', import.meta.url);
const source = name => readFileSync(new URL(name, directory), 'utf8')
    .replace(/^import .*;\n/gm, '').replace(/export class /g, 'class ');
let next = 0;
const signals = new Map(), timers = new Map();
const emitter = props => ({...props,
    connect(name, callback) { const id = ++next; signals.set(id, {object: this, name, callback}); return id; },
    disconnect(id) { assert(signals.delete(id)); },
});
const shield = emitter({active: false, locked: false}), overview = emitter({visible: false});
let workspaceIndex = 0;
const workspace = {index: () => workspaceIndex};
const manager = emitter({get_active_workspace: () => workspace});
const RGB = 2, RGBA = 3;
let actors = [];
let monotonic = 0;
const logs = [];
const SceneVisibility = vm.runInNewContext(`${source('scene-input.js')}\n${source('scene-visibility.js')}\nSceneVisibility;`, {
    Cogl: {TextureComponents: {RGB, RGBA}}, Meta: {WindowClientType: {WAYLAND: 0, X11: 1}, WindowType: {DESKTOP: 1}},
    Main: {screenShield: shield, overview},
    console: {log(message) { logs.push(message); }},
    global: {workspace_manager: manager, get_window_actors: () => actors},
    GLib: {PRIORITY_DEFAULT: 0, SOURCE_CONTINUE: true,
        get_monotonic_time: () => monotonic,
        timeout_add(_p, ms, callback) { assert.equal(ms, 500); const id = ++next; timers.set(id, callback); return id; },
        source_remove(id) { assert(timers.delete(id)); }},
});
const covered = (target, rects) => SceneVisibility.covered(target, rects);
assert(covered([0, 0, 10, 10], [[0, 0, 5, 10], [5, 0, 5, 10]]));
assert(!covered([0, 0, 10, 10], [[0, 0, 4, 10], [6, 0, 4, 10]]));
assert(!covered([0, 0, 10, 10], [[0, 0, 10, 9.99]]));
assert(!covered([0, 0, 10, 10], Array(129).fill([0, 0, 10, 10])));
assert(!covered([0, 0, 10, 10], [[NaN, 0, 10, 10]]));
for (let mask = 0; mask < 256; mask++) {
    const rects = Array.from({length: 8}, (_, i) => [i % 4, Math.floor(i / 4), 1, 1])
        .filter((_, i) => mask & (1 << i));
    assert.equal(covered([0, 0, 4, 2], rects), mask === 255);
}

function actor({components = RGB, opacity = 255, client = 0, minimized = false, windowType = 0,
    onWorkspace = true, visible = true, effect = false, transition = false, transformed = false,
    rect = {x: 100, y: 100, width: 900, height: 600}} = {}) {
    return {
        meta_window: {minimized, get_client_type: () => client, get_window_type: () => windowType,
            located_on_workspace: () => onWorkspace, showing_on_its_workspace: () => visible,
            get_buffer_rect: () => rect, get_frame_rect: () => rect},
        is_visible: () => visible, get_paint_opacity: () => opacity,
        get_transition: () => transition, get_effects: () => effect ? [{}] : [],
        get_texture: () => ({get_texture: () => ({is_simple: () => true,
            get_plane: () => ({get_components: () => components})})}),
        get_transformed_position: () => [rect.x + (transformed ? 30 : 0), rect.y],
        get_transformed_size: () => [rect.width, rect.height],
    };
}
assert(SceneVisibility.coveringRect(actor(), workspace));
assert(SceneVisibility.coveringRect(actor({components: RGBA}), workspace));
const noTextureQuery = actor();
noTextureQuery.get_texture = () => { throw new Error('texture queries are not needed'); };
assert(SceneVisibility.coveringRect(noTextureQuery, workspace));
for (const [options, reason] of [
    [{windowType: 1}, 'desktop-window'],
    [{opacity: 250}, 'actor-opacity'], [{client: 1}, 'non-wayland-client'],
    [{minimized: true}, 'minimized'], [{onWorkspace: false}, 'other-workspace'],
    [{visible: false}, 'not-showing'], [{effect: true}, 'actor-effects'],
    [{transition: true}, 'opacity-transition'], [{transformed: true}, 'geometry-mismatch']]) {
    const detail = {};
    assert.equal(SceneVisibility.coveringRect(actor(options), workspace, detail), null);
    assert.equal(detail.reason, reason);
}
for (const options of [{opacity: 250}, {client: 1}, {minimized: true},
    {onWorkspace: false}, {visible: false}, {effect: true}, {transition: true}, {transformed: true}])
    assert.equal(SceneVisibility.coveringRect(actor(options), workspace), null);

let snapshot = {version: 1, generation: 1, layout: {width: 1920, height: 1080,
    viewport: [0, 0, 1920, 1080], animation: [400, 300, 500, 300], pause_when_hidden: true, hotspots: []}};
let scene = {x: 0, y: 0, width: 1920, height: 1080};
const events = [], blocked = [];
const owned = actor();
const visibility = new SceneVisibility(() => snapshot, () => scene, a => a === owned,
    e => events.push(e), state => blocked.push(state));
visibility.start();
assert.equal(events.at(-1).hidden, false);
actors = [actor()];
visibility.poll(); assert.equal(events.at(-1).hidden, false, 'first coverage sample cannot suspend');
visibility.poll(); assert.equal(events.at(-1).hidden, true);
visibility.poll(); assert.equal(events.at(-1).hidden, true, 'heartbeat stays hidden');
actors = [actor({components: RGBA})];
visibility.poll(); assert.equal(events.at(-1).hidden, true, 'RGBA coverage follows the same geometric policy');
actors = [actor({components: RGBA, rect: {x: 100, y: 100, width: 500, height: 600}})];
visibility.poll(); assert.equal(events.at(-1).hidden, false, 'uncovered moving parts resume immediately');
actors = [actor({components: RGBA})];
visibility.poll(); assert.equal(events.at(-1).hidden, false, 'new coverage still requires two samples');
visibility.poll(); assert.equal(events.at(-1).hidden, true);
actors = [owned]; visibility.poll(); visibility.poll(); assert.equal(events.at(-1).hidden, false);
actors = [actor({windowType: 1})]; visibility.poll(); visibility.poll();
assert.equal(events.at(-1).hidden, false, 'a DING desktop must never suspend the animation');
actors = [actor()]; visibility.poll(); visibility.poll();
assert(events.at(-1).hidden);
workspaceIndex++;
const workspaceHandler = [...signals.values()].find(s => s.name === 'active-workspace-changed');
workspaceHandler.callback(); assert.equal(events.at(-1).hidden, false);
visibility.poll(); assert(events.at(-1).hidden);
snapshot = {...snapshot, generation: 2, layout: {...snapshot.layout, hotspots: [{id: 'next', rect: [500, 500, 10, 10]}]}};
visibility.poll(); assert(events.at(-1).hidden, 'input generation change alone must not oscillate coverage');
snapshot = {...snapshot, layout: {...snapshot.layout, animation: [1000, 300, 500, 300]}};
visibility.poll(); assert.equal(events.at(-1).hidden, false, 'geometry changes retire coverage');
overview.visible = true; visibility.poll(); assert(events.at(-1).hidden && !events.at(-1).locked);
overview.visible = false; shield.active = true;
visibility.poll(); assert(events.at(-1).locked);
snapshot.layout.pause_when_hidden = false;
visibility.poll(); assert(events.at(-1).locked, 'lock always blocks input even when source rendering is opted out');
shield.active = false; visibility.poll(); assert.equal(events.at(-1).hidden, false);
snapshot.layout.pause_when_hidden = true;
actors = [{meta_window: {get_client_type() { throw new Error('destroyed window'); }}}];
visibility.poll(); assert.equal(events.at(-1).hidden, false, 'destroy races fail open');
actors = Array(129).fill(actor()); visibility.poll(); assert.equal(events.at(-1).hidden, false);
scene = null; visibility.poll(); assert.equal(events.at(-1).hidden, false);
scene = {x: 100, y: 70, width: 2048, height: 1152};
snapshot.layout = {...snapshot.layout, width: 2048, height: 1120,
    viewport: [28, 0, 1991, 1120], animation: [900, 400, 600, 450]};
const target = SceneVisibility.target(snapshot, scene);
assert(target && target[0] > 900 && target[1] > 400, 'coverage uses the same constrained-source mapping as input');
snapshot.layout.animation = null; assert.equal(SceneVisibility.target(snapshot, scene), null);
const count = events.length;
// Diagnostics are bounded, identify actual method failures and never log titles.
assert(!logs.some(line => line.startsWith('Talis visibility diagnostic: ')), 'detailed coverage logging is opt-in');
visibility._diagnostics = true;
snapshot.layout.animation = [900, 400, 600, 450];
actors = [actor({components: RGBA})];
monotonic = 5_000_000; visibility.poll();
let diagnosticLogs = () => logs.filter(l => l.startsWith('Talis visibility diagnostic: '));
assert(diagnosticLogs().at(-1).includes('geometric-coverage'));
assert(diagnosticLogs().at(-1).includes('eligible'));
let emitted = diagnosticLogs().length;
actors = [actor()];
visibility.poll(); assert.equal(diagnosticLogs().length, emitted, 'changed polls still rate limited');
const broken = actor(); delete broken.get_transformed_position;
actors = [broken]; monotonic = 10_000_000; visibility.poll();
assert(diagnosticLogs().at(-1).includes('api-error'));
assert(diagnosticLogs().at(-1).includes('get_transformed_position'));
assert.equal(events.at(-1).hidden, false);
emitted = diagnosticLogs().length;
monotonic = 39_999_999; visibility.poll(); assert.equal(diagnosticLogs().length, emitted);
monotonic = 40_000_000; visibility.poll(); assert.equal(diagnosticLogs().length, emitted + 1);
actors = Array(100).fill(actor({components: RGBA}));
monotonic = 45_000_000; visibility.poll();
const summary = JSON.parse(diagnosticLogs().at(-1).split('Talis visibility diagnostic: ')[1]);
assert.equal(summary.windows.length, 16);
assert.equal(summary.counts.eligible, 100);
assert(diagnosticLogs().at(-1).length < 8192);
assert(!diagnosticLogs().at(-1).includes('title'));
visibility._diagnose = () => { throw new Error('unavailable diagnostic clock'); };
const beforeFailure = events.length;
visibility.poll();
assert.equal(events.length, beforeFailure + 1, 'diagnostic failure must not break heartbeat');
const finalCount = events.length;
visibility.stop(); visibility.stop(); visibility.poll();
assert(events.length >= count);
assert.equal(events.length, finalCount);
assert.equal(signals.size, 0); assert.equal(timers.size, 0);
console.log('PASS: exact union/holes, RGB/RGBA geometric coverage without texture queries, transforms, debounce, uncover resume, geometry/workspaces, lock/overview, uncertain/stale target, bounded polling and idempotent cleanup');
console.log('PASS: per-window rejection reasons and API stage, aggregate counts, 16-window/5s rate bounds, 30s heartbeat and privacy');
