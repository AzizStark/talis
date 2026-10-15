// The portable launcher exposes only Settings activation, never Shell eval.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
const source = readFileSync(new URL('../../integrations/gnome/extension.js', import.meta.url), 'utf8')
    .replace(/^import .*;\n/gm, '').replace('export default class ', 'class ');
let next = 0;
const signals = new Set();
const layout = {
    connect() { const id = ++next; signals.add(id); return id; },
    disconnect(id) { assert(signals.delete(id)); },
};
const stats = {opened: 0, stopped: 0, exported: 0, unexported: 0};
let methods;
const connection = {};
const Extension = class { constructor() { this.path = '/fixture'; } };
const LiveSurface = class {
    static available() { return true; }
    start() {}
    openSettings() { stats.opened++; }
    setScenes() {}
    stop() { stats.stopped++; }
};
const Talis = vm.runInNewContext(`${source}\nTalisDesktopExtension;`, {
    Extension, LiveSurface, Main: {layoutManager: layout}, console: {log() {}},
    Gio: {DBus: {session: connection}, DBusExportedObject: {
        wrapJSObject(xml, implementation) {
            assert(xml.includes('name="io.github.azizstark.Talis"'));
            assert.deepEqual(Object.keys(implementation), ['OpenSettings']);
            methods = implementation;
            return {
                export(bus, path) { assert.equal(bus, connection); assert.equal(path, '/io/github/azizstark/Talis'); stats.exported++; },
                unexport() { stats.unexported++; },
            };
        },
    }},
});
const extension = new Talis();
extension._rebuild = () => {};
extension.enable();
methods.OpenSettings();
assert.equal(stats.opened, 1);
assert.equal(stats.exported, 1);
extension.disable();
extension.disable();
assert.equal(stats.unexported, 1);
assert.equal(stats.stopped, 1);
assert.equal(signals.size, 0);
methods.OpenSettings();
assert.equal(stats.opened, 1, 'a stale reference cannot activate a disabled renderer');
