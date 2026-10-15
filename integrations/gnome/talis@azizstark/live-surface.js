import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import {SceneInput} from './scene-input.js';
import {SceneVisibility} from './scene-visibility.js';

// GNOME 48 owned surface transport. No frame readback or image-file polling.
// The compositor owns the Wayland buffers; a Clone references their actor.
export class LiveSurface {
    constructor(path) {
        this._scene = LiveSurface.sceneSource(path);
        this._binary = LiveSurface.binary(path);
        this._scenes = [];
        this._overlays = [];
        this._signals = [];
        this._stopped = false;
        this._concealing = false;
        this._frames = 0;
        this._ready = false;
        this._writes = [];
        this._sequence = 0;
        this._writing = false;
        this._inputLayout = null;
    }

    static sceneSource(path) {
        const file = Gio.File.new_for_path(GLib.build_filenamev([path, 'scene.json']));
        if (!file.query_exists(null))
            return null;
        const [ok, bytes] = file.load_contents(null);
        if (!ok || bytes.length > 4096)
            throw new Error('Invalid or oversized scene configuration');
        const config = JSON.parse(new TextDecoder().decode(bytes));
        if (config.version !== 3 || !['day', 'night'].includes(config.scene)
            || !['music', 'demo', 'static'].includes(config.mode))
            throw new Error('Unsupported scene configuration');
        const packagedAssets = GLib.build_filenamev([path, 'assets']);
        const assets = GLib.file_test(packagedAssets, GLib.FileTest.IS_DIR)
            ? packagedAssets : GLib.build_filenamev([path, '..', '..', '..', 'assets']);
        return {scene: config.scene, mode: config.mode,
            assets};
    }

    static available(path) {
        const scene = LiveSurface.sceneSource(path);
        if (!scene) return false;
        return GLib.file_test(LiveSurface.binary(path),
            GLib.FileTest.IS_EXECUTABLE);
    }

    static binary(path) {
        const name = 'talis-gnome-scene';
        const packaged = GLib.build_filenamev([path, name]);
        if (GLib.file_test(packaged, GLib.FileTest.IS_EXECUTABLE)) return packaged;
        return GLib.build_filenamev([path, '..', '..', '..', 'target', 'release', name]);
    }

    start(scenes) {
        this.setScenes(scenes);
        const launcher = new Gio.SubprocessLauncher({
            flags: Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_MERGE
                | (this._scene ? Gio.SubprocessFlags.STDIN_PIPE : 0),
        });
        launcher.setenv('GDK_BACKEND', 'wayland', true);
        for (const variable of ['LD_PRELOAD', 'GDK_DEBUG', 'GDK_DISABLE'])
            launcher.unsetenv(variable);
        try {
            this._client = Meta.WaylandClient.new(global.context, launcher);
            this._mapId = global.window_manager.connect_after('map', (_wm, actor) => {
                try {
                    this._attach(actor);
                } catch (error) {
                    console.error(`Talis renderer: attach failed: ${error.message}`);
                    this.stop();
                }
            });
            const argv = [this._binary];
            if (this._scene) {
                if (scenes.length !== 1)
                    throw new Error('The real-scene milestone currently requires exactly one monitor');
                const {width, height} = scenes[0];
                if (![width, height].every(n => Number.isInteger(n) && n >= 1 && n <= 8192))
                    throw new Error('Monitor geometry is outside the scene limits');
                argv.push('--asset-root', this._scene.assets, '--scene', this._scene.scene,
                    '--mode', this._scene.mode, '--size', `${width}x${height}`);
            }
            this._process = this._client.spawnv(global.display, argv);
        } finally {
            launcher.close();
        }
        this._cancel = new Gio.Cancellable();
        if (this._scene) {
            this._inputPipe = this._process.get_stdin_pipe();
            this._input = new SceneInput(event => this._sendInput(event), () => this.openSettings());
            this._signals.push([global.display, global.display.connect('notify::focus-window', () => this._input?.cancel())]);
            this._visibility = new SceneVisibility(() => this._inputLayout,
                () => this._ready && this._scenes.length === 1 ? this._scenes[0] : null,
                actor => actor === this._source, event => this._sendInput(event),
                blocked => {
                    if (this._inputBlocked === blocked) return;
                    this._inputBlocked = blocked;
                    this._updateInput();
                });
            this._visibility.start();
        }
        this._output = new Gio.DataInputStream({base_stream: this._process.get_stdout_pipe()});
        this._readOutput();
        this._process.wait_async(null, (process, result) => {
            try {
                process.wait_finish(result);
                if (!this._stopped) {
                    console.warn('Talis renderer: renderer exited; static scene remains');
                    this.stop();
                }
            } catch (error) {
                console.warn(`Talis renderer: process wait: ${error.message}`);
            }
        });
        this._startupTimeout = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, this._scene ? 60 : 15, () => {
            this._startupTimeout = 0;
            if (!this._source || (this._scene && !this._ready)) {
                console.warn('Talis renderer: owned surface/first rendered scene did not arrive in time');
                this.stop();
            }
            return GLib.SOURCE_REMOVE;
        });
    }

    _readOutput() {
        if (this._stopped || !this._output)
            return;
        this._output.read_line_async(GLib.PRIORITY_DEFAULT, this._cancel, (stream, result) => {
            try {
                const [line] = stream.read_line_finish_utf8(result);
                if (this._stopped || line === null)
                    return;
                if (line.startsWith('INPUT ')) {
                    if (line.length > 4096)
                        throw new Error('Oversized input layout');
                    const snapshot = JSON.parse(line.slice(6));
                    SceneInput.validate(snapshot);
                    this._inputLayout = snapshot;
                    this._updateInput();
                }
                const match = /^FRAME ([0-9]+)$/.exec(line);
                if (match) {
                    this._frames = Number(match[1]);
                    if (this._scene && this._frames > 0 && !this._ready) {
                        this._ready = true;
                        this.setScenes(this._scenes);
                    }
                }
                console.log(`Talis renderer: ${line}`);
                this._readOutput();
            } catch (error) {
                if (!this._stopped) {
                    console.warn(`Talis renderer: renderer output: ${error.message}`);
                    this.stop();
                }
            }
        });
    }

    _attach(actor) {
        if (this._stopped || !this._client.owns_window(actor.meta_window))
            return;
        const window = actor.meta_window;
        if (this._scene && window.get_title() === 'Talis · Settings') {
            if (this._settingsWindow !== window) {
                this._settingsWindow = window;
                this._signals.push([window, window.connect('unmanaged', () => {
                    if (this._settingsWindow === window)
                        this._settingsWindow = null;
                })]);
            }
            if (this._settingsRequested) {
                this._settingsRequested = false;
                window.activate(global.get_current_time());
            }
            return;
        }
        if (this._source || (this._scene && window.get_title() !== 'Talis · Scene'))
            return;
        this._source = actor;
        this._signals.push([window, window.connect('notify::minimized', () => {
            if (!this._stopped && !window.minimized)
                this._ensureConcealed(window);
        })]);
        this._signals.push([window, window.connect_after('shown', () => {
            if (!this._stopped)
                this._ensureConcealed(window);
        })]);
        this._signals.push([actor, actor.connect('destroy', () => {
            // The process has only one test surface. Do not restart in a loop.
            this.stop();
        })]);
        if (!this._ensureConcealed(window))
            return;
        this.setScenes(this._scenes);
        console.log('Talis renderer: owned surface concealed; desktop presentation only');
    }

    _ensureConcealed(window) {
        if (this._stopped || this._concealing)
            return false;
        this._concealing = true;
        try {
            if (!window.minimized) {
                // Mutter 48's skip-taskbar flag disables can_minimize(). Restore
                // it before minimizing; hiding from lists must happen LAST.
                // This also handles an external unminimize without a retry loop.
                this._client.show_in_window_list(window);
                if (!window.can_minimize())
                    throw new Error('Owned renderer window does not permit minimization');
                window.minimize();
                if (!window.minimized)
                    throw new Error('Owned renderer window did not become minimized');
            }
            this._client.hide_from_window_list(window);
            return true;
        } catch (error) {
            console.error(`Talis renderer: concealment failed: ${error.message}; static scene remains`);
            this.stop();
            return false;
        } finally {
            this._concealing = false;
        }
    }

    setScenes(scenes) {
        this._input?.clear();
        for (const {clone} of this._overlays) {
            clone?.destroy();
        }
        this._overlays = [];
        this._scenes = scenes;
        if (this._stopped)
            return;
        for (const scene of scenes) {
            const width = this._scene ? scene.width : Math.min(416, Math.max(1, scene.width - 48));
            const height = this._scene ? scene.height : width * 234 / 416;
            const x = this._scene ? 0 : Math.max(0, scene.width - width - 24);
            const y = this._scene ? 0 : Math.max(48, scene.height - height - 48);
            const clone = this._source && (!this._scene || this._ready) ? new Clutter.Clone({
                source: this._source, reactive: false, x, y, width, height,
            }) : null;
            if (clone)
                // Preserve the static first-frame fallback beneath the clone.
                scene.insert_child_at_index(clone, 1);
            this._overlays.push({clone});
        }
        this._updateInput();
    }

    _updateInput() {
        if (!this._input || !this._inputLayout)
            return;
        const scene = this._ready && this._source && this._scenes.length === 1 ? this._scenes[0] : null;
        const mapping = SceneInput.presentation(this._inputLayout, scene);
        const clone = this._overlays[0]?.clone;
        if (mapping && clone) {
            for (const property of ['x', 'y', 'width', 'height'])
                clone[property] = mapping[property];
        }
        this._input.update(this._inputLayout, this._inputBlocked ? null : scene);
    }

    openSettings() {
        if (!this._scene || this._stopped || !this._inputPipe || this._visibility?.locked)
            return;
        this._settingsRequested = true;
        this._input?.cancel();
        this._sendInput({event: 'settings'});
        if (this._settingsWindow) {
            try {
                this._settingsWindow.activate(global.get_current_time());
                this._settingsRequested = false;
            } catch {
                // Hidden GTK windows can lose their old compositor toplevel.
                // The subsequent owned Settings map is authoritative instead.
                this._settingsWindow = null;
            }
        }
    }

    _sendInput(event) {
        if (this._stopped || !this._inputPipe)
            return;
        const next = {...event, seq: ++this._sequence};
        if (event.event === 'visibility')
            this._writes = this._writes.filter(queued => queued.event !== 'visibility');
        if (event.event === 'motion' && this._writes.at(-1)?.event === 'motion')
            this._writes[this._writes.length - 1] = next;
        else if (this._writes.length < 32)
            this._writes.push(next);
        else {
            // Never retain an unbounded backlog or deliver a delayed click burst.
            this._writes = [{event: 'cancel', seq: ++this._sequence}];
            this._input?.cancel();
            if (event.event === 'visibility')
                this._writes.push({...event, seq: ++this._sequence});
        }
        this._flushInput();
    }

    _flushInput() {
        if (this._stopped || this._writing || !this._writes.length)
            return;
        const bytes = new TextEncoder().encode(`${JSON.stringify(this._writes.shift())}\n`);
        this._writing = true;
        this._inputPipe.write_all_async(bytes, GLib.PRIORITY_DEFAULT, this._cancel, (stream, result) => {
            this._writing = false;
            if (this._stopped)
                return;
            try {
                stream.write_all_finish(result);
                this._flushInput();
            } catch (error) {
                console.warn(`Talis renderer: input pipe failed: ${error.message}`);
                this.stop();
            }
        });
    }

    stop() {
        if (this._stopped)
            return;
        this._stopped = true;
        this._input?.stop();
        this._input = null;
        this._visibility?.stop();
        this._visibility = null;
        this._writes = [];
        if (this._startupTimeout) {
            GLib.source_remove(this._startupTimeout);
            this._startupTimeout = 0;
        }
        if (this._mapId) {
            global.window_manager.disconnect(this._mapId);
            this._mapId = 0;
        }
        for (const [object, id] of this._signals) {
            try {
                object.disconnect(id);
            } catch { /* A destroyed source may already have dropped its handlers. */ }
        }
        this._signals = [];
        this.setScenes([]);
        this._cancel?.cancel();
        if (this._process) {
            try {
                this._process.send_signal(15);
            } catch { /* It may have already exited. */ }
        }
        this._source = null;
        this._client = null;
        this._process = null;
        this._output = null;
        this._inputPipe = null;
        this._settingsWindow = null;
    }
}
