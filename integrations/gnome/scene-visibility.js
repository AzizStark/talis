import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {SceneInput} from './scene-input.js';

// No pixel readback, foreign-client IPC or per-frame polling.
export class SceneVisibility {
    constructor(snapshot, scene, owns, send, inputBlocked, diagnostics = false) {
        Object.assign(this, {_snapshot: snapshot, _scene: scene, _owns: owns,
            _send: send, _inputBlocked: inputBlocked, _diagnostics: diagnostics});
        this._signals = [];
        this._previous = null;
        this._count = 0;
        this._stopped = false;
    }

    start() {
        for (const [object, signal] of [[Main.screenShield, 'active-changed'],
            [Main.screenShield, 'locked-changed'], [Main.overview, 'showing'],
            [Main.overview, 'hidden'], [global.workspace_manager, 'active-workspace-changed']]) {
            this._signals.push([object, object.connect(signal, () => {
                this._previous = null;
                this._count = 0;
                this.poll();
            })]);
        }
        this._timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 500, () => {
            this.poll();
            return GLib.SOURCE_CONTINUE;
        });
        this.poll();
    }

    get locked() {
        return Boolean(Main.screenShield.active || Main.screenShield.locked);
    }

    // Subtract the union exactly: bounding boxes must never cover visible holes.
    static covered(target, rects) {
        if (!SceneVisibility.rect(target) || rects.length > 128)
            return false;
        let pieces = [target];
        for (const r of rects) {
            if (!SceneVisibility.rect(r))
                return false;
            const next = [];
            for (const [x, y, w, h] of pieces) {
                const l = Math.max(x, r[0]), t = Math.max(y, r[1]);
                const right = Math.min(x + w, r[0] + r[2]), b = Math.min(y + h, r[1] + r[3]);
                if (right <= l || b <= t) {
                    next.push([x, y, w, h]);
                    continue;
                }
                if (t > y) next.push([x, y, w, t - y]);
                if (b < y + h) next.push([x, b, w, y + h - b]);
                if (l > x) next.push([x, t, l - x, b - t]);
                if (right < x + w) next.push([right, t, x + w - right, b - t]);
            }
            if (next.length > 512)
                return false;
            pieces = next;
            if (!pieces.length)
                return true;
        }
        return false;
    }

    static rect(r) {
        return Array.isArray(r) && r.length === 4 && r.every(Number.isFinite)
            && r[2] > 0 && r[3] > 0;
    }

    static target(snapshot, scene) {
        const mapping = SceneInput.presentation(snapshot, scene);
        if (!mapping || !SceneVisibility.rect(snapshot?.layout?.animation))
            return null;
        const [x, y, w, h] = snapshot.layout.animation;
        if (x < 0 || y < 0 || x + w > snapshot.layout.width || y + h > snapshot.layout.height)
            return null;
        const l = Math.max(0, mapping.x + x * mapping.scaleX);
        const t = Math.max(0, mapping.y + y * mapping.scaleY);
        const right = Math.min(scene.width, mapping.x + (x + w) * mapping.scaleX);
        const b = Math.min(scene.height, mapping.y + (y + h) * mapping.scaleY);
        const rect = [scene.x + l, scene.y + t, right - l, b - t];
        return SceneVisibility.rect(rect) ? rect : null;
    }

    // Coverage is geometric, not a proof of opacity. Per the chosen GNOME
    // policy, alpha-bearing windows can suspend animation underneath them.
    static coveringRect(actor, workspace, detail = {}) {
        const reject = reason => { detail.reason = reason; return null; };
        detail.stage = 'meta_window';
        const window = actor.meta_window;
        // Native Wayland only: X11 shapes/masks are not exposed by this API.
        if (!window) return reject('no-window');
        // DING's transparent, fullscreen desktop is not an occluding app.
        if (window.get_window_type() === Meta.WindowType.DESKTOP) return reject('desktop-window');
        if (Meta.WindowClientType?.WAYLAND === undefined) return reject('client-enum-unavailable');
        detail.stage = 'get_client_type';
        if (window.get_client_type() !== Meta.WindowClientType.WAYLAND) return reject('non-wayland-client');
        if (window.minimized) return reject('minimized');
        detail.stage = 'located_on_workspace';
        if (!window.located_on_workspace(workspace)) return reject('other-workspace');
        detail.stage = 'showing_on_its_workspace';
        if (!window.showing_on_its_workspace()) return reject('not-showing');
        detail.stage = 'is_visible';
        if (!actor.is_visible()) return reject('actor-hidden');
        detail.stage = 'get_paint_opacity';
        const opacity = actor.get_paint_opacity();
        detail.opacity = Number.isFinite(opacity) ? opacity : null;
        if (opacity !== 255) return reject('actor-opacity');
        detail.stage = 'get_transition';
        if (actor.get_transition('opacity')) return reject('opacity-transition');
        detail.stage = 'get_effects';
        if (actor.get_effects()?.length) return reject('actor-effects');
        detail.stage = 'get_buffer_rect';
        const buffer = window.get_buffer_rect();
        detail.stage = 'get_transformed_position';
        const [ax, ay] = actor.get_transformed_position();
        detail.stage = 'get_transformed_size';
        const [aw, ah] = actor.get_transformed_size();
        if (![ax, ay, aw, ah].every(Number.isFinite)) return reject('nonfinite-transform');
        const delta = [ax - buffer.x, ay - buffer.y, aw - buffer.width, ah - buffer.height];
        detail.delta = delta.map(n => Number.isFinite(n) ? Math.round(n * 100) / 100 : null);
        if (Math.abs(ax - buffer.x) > 1 || Math.abs(ay - buffer.y) > 1
            || Math.abs(aw - buffer.width) > 1 || Math.abs(ah - buffer.height) > 1)
            return reject('geometry-mismatch'); // Workspace/window animations or uncertain transforms.
        detail.stage = 'get_frame_rect';
        const frame = window.get_frame_rect();
        // Exclude decorations/shadows and the edge sampling footprint.
        const l = Math.max(frame.x, buffer.x) + 2;
        const t = Math.max(frame.y, buffer.y) + 2;
        const r = Math.min(frame.x + frame.width, buffer.x + buffer.width) - 2;
        const b = Math.min(frame.y + frame.height, buffer.y + buffer.height) - 2;
        const rect = [l, t, r - l, b - t];
        if (!SceneVisibility.rect(rect)) return reject('empty-interior');
        detail.reason = 'eligible';
        return rect;
    }

    _diagnose(diagnostic) {
        if (!this._diagnostics)
            return;
        // Maximum one bounded summary per five seconds, with a 30-second
        // heartbeat for unchanged results. No titles, URLs, paths or app data.
        const now = GLib.get_monotonic_time();
        const key = JSON.stringify(diagnostic);
        const elapsed = now - (this._diagnosticAt ?? -Infinity);
        if (elapsed < 5_000_000 || (key === this._diagnosticKey && elapsed < 30_000_000))
            return;
        this._diagnosticAt = now;
        this._diagnosticKey = key;
        console.log(`Talis visibility diagnostic: ${key}`);
    }

    poll() {
        if (this._stopped)
            return;
        const snapshot = this._snapshot();
        if (!snapshot)
            return;
        let locked = false, hidden = false;
        const diagnostic = {policy: 'geometric-coverage', reason: 'unknown', windows: [], counts: {}};
        let detail = null;
        try {
            locked = this.locked;
            if (locked || Main.overview.visible) {
                diagnostic.reason = locked ? 'screen-shield' : 'overview';
                hidden = true;
                this._previous = null;
                this._count = 0;
            } else if (snapshot.layout.pause_when_hidden) {
                diagnostic.reason = 'coverage-check';
                const target = SceneVisibility.target(snapshot, this._scene());
                const workspace = global.workspace_manager.get_active_workspace();
                const actors = global.get_window_actors();
                diagnostic.target = target;
                diagnostic.actors = actors.length;
                if (target && actors.length <= 128) {
                    const rects = [];
                    let index = 0;
                    for (const actor of actors) {
                        detail = {index: index++};
                        if (!this._owns(actor)) {
                            const rect = SceneVisibility.coveringRect(actor, workspace, detail);
                            if (rect) rects.push(rect);
                        } else {
                            detail.reason = 'owned-source';
                        }
                        diagnostic.counts[detail.reason] = (diagnostic.counts[detail.reason] ?? 0) + 1;
                        if (diagnostic.windows.length < 16) diagnostic.windows.push(detail);
                    }
                    const key = JSON.stringify([workspace.index(), target, rects]);
                    const covered = SceneVisibility.covered(target, rects);
                    this._count = covered ? (key === this._previous ? this._count + 1 : 1) : 0;
                    this._previous = key;
                    hidden = this._count >= 2;
                    diagnostic.reason = hidden ? 'covered' : covered ? 'coverage-confirming' : 'coverage-incomplete';
                } else {
                    diagnostic.reason = !target ? 'no-animation-target' : 'too-many-windows';
                    this._previous = null;
                    this._count = 0;
                }
            } else {
                diagnostic.reason = 'suspension-disabled';
                this._previous = null;
                this._count = 0;
            }
        } catch {
            // Missing APIs / destroyed windows are ordinary uncertainty, not a
            // reason to freeze a visible desktop or throw inside Shell's loop.
            this._previous = null;
            this._count = 0;
            diagnostic.reason = 'probe-error';
            if (detail) {
                detail.reason = 'api-error';
                diagnostic.counts['api-error'] = (diagnostic.counts['api-error'] ?? 0) + 1;
                if (!diagnostic.windows.includes(detail) && diagnostic.windows.length < 16)
                    diagnostic.windows.push(detail);
            }
        }
        diagnostic.hidden = hidden;
        diagnostic.locked = locked;
        try {
            this._diagnose(diagnostic);
        } catch { /* Optional diagnostics must never interrupt the visibility heartbeat. */ }
        const status = `${locked ? 'locked' : hidden ? 'hidden' : 'visible'}`;
        if (this._status !== status) {
            this._status = status;
            console.log(`Talis visibility: ${status}`);
        }
        this._inputBlocked(hidden);
        this._send({event: 'visibility', generation: snapshot.generation, hidden, locked});
    }

    stop() {
        if (this._stopped) return;
        this._stopped = true;
        if (this._timer) GLib.source_remove(this._timer);
        this._timer = 0;
        for (const [object, id] of this._signals) object.disconnect(id);
        this._signals = [];
    }
}
