import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import St from 'gi://St';

// Only ellipse scanlines are reactive, not a fullscreen transparent input plane.
// Commands/availability remain authoritative in Rust, including stale gestures.
export class SceneInput {
    constructor(send, openSettings) {
        this._send = send;
        this._openSettings = openSettings;
        this._actors = [];
        this._capture = null;
        this._captureTimeout = 0;
        this._generation = 0;
        this._hovered = false;
        this._stageId = global.stage.connect('captured-event', (_stage, event) => {
            if (!this._capture)
                return Clutter.EVENT_PROPAGATE;
            const point = this._point(event, this._capture.mapping);
            if (!point) {
                this.cancel();
                return Clutter.EVENT_PROPAGATE;
            }
            if (event.type() === Clutter.EventType.BUTTON_RELEASE && event.get_button() === 1) {
                const capture = this._capture;
                this._send({event: 'release', generation: this._capture.generation, ...point});
                this._endCapture();
                const [x, y, w, h] = capture.rect;
                if (capture.id === 'settings'
                    && ((point.x - x - w / 2) / (w / 2)) ** 2
                        + ((point.y - y - h / 2) / (h / 2)) ** 2 <= 1)
                    this._openSettings();
            } else if (event.type() === Clutter.EventType.MOTION) {
                const [x, y, w, h] = this._capture.rect;
                if (((point.x - x - w / 2) / (w / 2)) ** 2
                    + ((point.y - y - h / 2) / (h / 2)) ** 2 > 1) {
                    this.cancel();
                    return Clutter.EVENT_PROPAGATE;
                }
                this._send({event: 'motion', generation: this._capture.generation, ...point});
            } else if (event.type() === Clutter.EventType.BUTTON_PRESS) {
                this.cancel();
            }
            // Never grab keyboard/pointer or prevent another app's event delivery.
            return Clutter.EVENT_PROPAGATE;
        });
    }

    static validate(snapshot) {
        const {version, generation, layout} = snapshot;
        if (version !== 1 || !Number.isSafeInteger(generation) || generation < 1 || !layout
            || ![layout.width, layout.height].every(n => Number.isInteger(n) && n >= 0 && n <= 8192)
            || !Array.isArray(layout.hotspots) || layout.hotspots.length > 5)
            throw new Error('Invalid Shell input layout');
        if (layout.viewport !== undefined) {
            const v = layout.viewport;
            if (!Array.isArray(v) || v.length !== 4 || !v.every(Number.isFinite)
                || v[0] < 0 || v[1] < 0 || v[2] <= 0 || v[3] <= 0
                || v[0] + v[2] > layout.width || v[1] + v[3] > layout.height)
                throw new Error('Invalid Shell scene viewport');
        }
        const ids = new Set();
        for (const {id, rect} of layout.hotspots) {
            if (!['play_pause', 'previous', 'next', 'volume', 'settings'].includes(id) || ids.has(id)
                || !Array.isArray(rect) || rect.length !== 4 || !rect.every(Number.isFinite))
                throw new Error('Invalid Shell hotspot');
            const [x, y, w, h] = rect;
            if (x < 0 || y < 0 || w <= 0 || h <= 0 || w > 512 || h > 512
                || x + w > layout.width + 1 || y + h > layout.height + 1)
                throw new Error('Shell hotspot outside source');
            ids.add(id);
        }
    }

    _point(event, mapping) {
        const [x, y] = event.get_coords();
        const [ok, localX, localY] = mapping.scene.transform_stage_point(x, y);
        if (!ok || ![localX, localY].every(Number.isFinite))
            return null;
        const point = {x: (localX - mapping.x) / mapping.scaleX,
            y: (localY - mapping.y) / mapping.scaleY};
        return [point.x, point.y].every(Number.isFinite) ? point : null;
    }

    _setCursor(cursor) {
        // GNOME 48 uses POINTER, not POINTING_HAND. GJS can coerce a missing
        // enum member to 0 (INVALID), which aborts Mutter's native cursor loader.
        // A try/catch cannot recover from that abort: validate before calling.
        if (!Number.isInteger(cursor) || cursor <= 1 || cursor === Meta.Cursor.NONE)
            return false;
        global.display.set_cursor(cursor);
        return true;
    }

    static presentation(snapshot, scene) {
        const {layout} = snapshot;
        if (!scene || ![layout.width, layout.height, scene.width, scene.height]
            .every(n => Number.isFinite(n) && n > 0 && n <= 8192))
            return null;
        const [vx, vy, vw, vh] = layout.viewport ?? [0, 0, layout.width, layout.height];
        // Fill the desktop with the content, not with the source's letterbox.
        // Preserve aspect ratio; the scene's clip trims overflow at its edges.
        const scale = Math.max(scene.width / vw, scene.height / vh);
        return {scene, scaleX: scale, scaleY: scale,
            x: (scene.width - vw * scale) / 2 - vx * scale,
            y: (scene.height - vh * scale) / 2 - vy * scale,
            width: layout.width * scale, height: layout.height * scale};
    }

    update(snapshot, scene) {
        SceneInput.validate(snapshot);
        this.clear();
        this._generation = snapshot.generation;
        const {layout} = snapshot;
        // Mutter can constrain the owned source to the work area. Clutter.Clone
        // scales that source into the full scene allocation: apply the same
        // mapping to input instead of dropping all controls on a size mismatch.
        const mapping = SceneInput.presentation(snapshot, scene);
        if (!mapping)
            return;
        console.log(`Talis input: mapped ${layout.hotspots.length} controls from ${layout.width}x${layout.height} to ${scene.width}x${scene.height}`);
        for (const hotspot of layout.hotspots) {
            const [sourceX, sourceY, sourceW, sourceH] = hotspot.rect;
            const x = mapping.x + sourceX * mapping.scaleX;
            const y = mapping.y + sourceY * mapping.scaleY;
            const w = sourceW * mapping.scaleX;
            const h = sourceH * mapping.scaleY;
            const group = new Clutter.Actor({reactive: false, width: scene.width, height: scene.height});
            scene.add_child(group);
            this._actors.push(group);
            for (let row = Math.max(0, Math.floor(y)); row < Math.min(scene.height, Math.ceil(y + h)); row++) {
                const dy = (row + 0.5 - y - h / 2) / (h / 2);
                if (Math.abs(dy) > 1)
                    continue;
                const dx = w / 2 * Math.sqrt(1 - dy * dy);
                const left = Math.max(0, Math.ceil(x + w / 2 - dx - 0.5));
                const right = Math.min(Math.floor(scene.width) - 1, Math.floor(x + w / 2 + dx - 0.5));
                if (right < left)
                    continue;
                const span = new St.Widget({reactive: true, can_focus: false,
                    x: left, y: row, width: right - left + 1, height: 1});
                group.add_child(span);
                span.connect('enter-event', () => {
                    this._hovered = this._setCursor(Meta.Cursor.POINTER);
                    return Clutter.EVENT_PROPAGATE;
                });
                span.connect('leave-event', () => {
                    this._hovered = false;
                    this._setCursor(Meta.Cursor.DEFAULT);
                    return Clutter.EVENT_PROPAGATE;
                });
                span.connect('button-press-event', (_actor, event) => {
                    if (hotspot.id === 'volume' || event.get_button() !== 1)
                        return Clutter.EVENT_PROPAGATE;
                    const point = this._point(event, mapping);
                    if (!point)
                        return Clutter.EVENT_PROPAGATE;
                    this._capture = {generation: this._generation, mapping,
                        id: hotspot.id, rect: hotspot.rect};
                    // Mutter can route a release over a client directly to that
                    // client, bypassing Shell's capture signal. Bound a lost
                    // gesture without taking a global pointer/keyboard grab.
                    this._captureTimeout = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 5, () => {
                        this._captureTimeout = 0;
                        this.cancel();
                        return GLib.SOURCE_REMOVE;
                    });
                    this._send({event: 'press', generation: this._generation, ...point});
                    return Clutter.EVENT_STOP;
                });
                span.connect('scroll-event', (_actor, event) => {
                    if (hotspot.id !== 'volume')
                        return Clutter.EVENT_PROPAGATE;
                    let dy;
                    switch (event.get_scroll_direction()) {
                    case Clutter.ScrollDirection.UP: dy = -1; break;
                    case Clutter.ScrollDirection.DOWN: dy = 1; break;
                    case Clutter.ScrollDirection.SMOOTH: [, dy] = event.get_scroll_delta(); break;
                    default: return Clutter.EVENT_PROPAGATE;
                    }
                    if (!Number.isFinite(dy) || dy === 0)
                        return Clutter.EVENT_PROPAGATE;
                    const point = this._point(event, mapping);
                    if (!point)
                        return Clutter.EVENT_PROPAGATE;
                    this._send({event: 'scroll', generation: this._generation,
                        ...point, dy: Math.max(-20, Math.min(20, dy))});
                    return Clutter.EVENT_STOP;
                });
            }
        }
    }

    cancel() {
        if (this._capture)
            this._send({event: 'cancel'});
        this._endCapture();
    }

    _endCapture() {
        this._capture = null;
        if (this._captureTimeout) {
            GLib.source_remove(this._captureTimeout);
            this._captureTimeout = 0;
        }
    }

    clear() {
        this.cancel();
        for (const actor of this._actors)
            actor.destroy();
        this._actors = [];
        if (this._hovered) {
            this._setCursor(Meta.Cursor.DEFAULT);
            this._hovered = false;
        }
    }

    stop() {
        this.clear();
        if (this._stageId) {
            global.stage.disconnect(this._stageId);
            this._stageId = 0;
        }
    }
}
