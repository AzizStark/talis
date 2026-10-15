import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GdkPixbuf from 'gi://GdkPixbuf';
import GLib from 'gi://GLib';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {LiveSurface} from './live-surface.js';

// GNOME 48 desktop presentation. The renderer owns music and controls.
// No wallpaper changes. Shell's hierarchy is not public API.
export default class TalisDesktopExtension extends Extension {
    enable() {
        this._actors = [];
        try {
            this._monitorsChangedId = Main.layoutManager.connect(
                'monitors-changed', () => this._rebuild());
            this._startupCompleteId = Main.layoutManager.connect(
                'startup-complete', () => this._rebuild());
            this._rebuild();
            if (LiveSurface.available(this.path)) {
                this._live = new LiveSurface(this.path);
                this._live.start(this._actors);
            }
            console.log('Talis: extension enabled');
        } catch (error) {
            this.disable();
            throw error;
        }
    }

    disable() {
        this._live?.stop();
        this._live = null;
        if (this._startupCompleteId) {
            Main.layoutManager.disconnect(this._startupCompleteId);
            this._startupCompleteId = 0;
        }
        if (this._monitorsChangedId) {
            Main.layoutManager.disconnect(this._monitorsChangedId);
            this._monitorsChangedId = 0;
        }
        this._destroyActors();
    }

    _destroyActors() {
        this._live?.setScenes([]);
        for (const actor of this._actors ?? [])
            actor.destroy();
        this._actors = [];
    }

    _rebuild() {
        this._destroyActors();

        const backgroundGroup = Main.layoutManager._backgroundGroup;
        if (!backgroundGroup || backgroundGroup.get_parent() !== global.window_group) {
            throw new Error('Talis: GNOME background group is unavailable');
        }

        const preset = LiveSurface.sceneSource(this.path);
        const imagePath = preset ? GLib.build_filenamev([
            preset.assets, 'scenes', '1440p', preset.scene, 'beauty.png',
        ]) : GLib.build_filenamev([this.path, 'beauty.png']);
        const imageFile = Gio.File.new_for_path(imagePath);
        const usableImage = imageFile?.query_exists(null) ?? false;
        const [format, imageWidth, imageHeight] = usableImage
            ? GdkPixbuf.Pixbuf.get_file_info(imagePath) : [null, 0, 0];
        if (usableImage && (!format || imageWidth <= 0 || imageHeight <= 0))
            throw new Error(`Talis: image cannot be read: ${imagePath}`);

        for (const monitor of Main.layoutManager.monitors) {
            const actor = new Clutter.Actor({
                reactive: false,
                x: monitor.x,
                y: monitor.y,
                width: monitor.width,
                height: monitor.height,
            });
            // Register immediately so a failed image load cannot leak an actor.
            this._actors.push(actor);
            actor.set_clip_to_allocation(true);

            if (usableImage) {
                const scale = Math.max(monitor.width / imageWidth,
                    monitor.height / imageHeight);
                const width = Math.ceil(imageWidth * scale);
                const height = Math.ceil(imageHeight * scale);
                const imagePlane = new Clutter.Actor({
                    reactive: false,
                    layout_manager: new Clutter.BinLayout(),
                    x: (monitor.width - width) / 2,
                    y: (monitor.height - height) / 2,
                    width,
                    height,
                });
                actor.add_child(imagePlane);
                const image = St.TextureCache.get_default().load_file_async(
                    imageFile, width, height, 1, 1.0);
                // Allocation owns the image size even after async loading updates
                // its natural dimensions. Preserve aspect and crop to fill.
                image.x_expand = true;
                image.y_expand = true;
                image.x_align = Clutter.ActorAlign.FILL;
                image.y_align = Clutter.ActorAlign.FILL;
                image.reactive = false;
                imagePlane.add_child(image);
            } else {
                const fallback = new St.Widget({
                    reactive: false,
                    x: 0,
                    y: 0,
                    width: monitor.width,
                    height: monitor.height,
                    style: 'background-color: #58402f;',
                });
                actor.add_child(fallback);
            }

            // Mutter lowers recognized backgrounds and managed window actors
            // during workspace restacking. A plain sibling in window_group is
            // left above them. Keep the scene inside the recognized background
            // group so its entire subtree stays below application windows.
            backgroundGroup.add_child(actor);
            backgroundGroup.set_child_above_sibling(actor, null);
            console.log(`Talis: scene placed ${monitor.width}x${monitor.height} at ${monitor.x},${monitor.y}`);
        }
        this._live?.setScenes(this._actors);
    }
}
