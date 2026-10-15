// Temporary read-only script. No window mutations, global settings or timers.
// The owned app delays Snapshot replies by 500 ms to pace this callback chain.
var talisService = __SERVICE__;
var talisPath = '/io/github/azizstark/Talis/Visibility';
var talisInterface = 'io.github.azizstark.Talis.Visibility';
var talisPrevious = null;
var talisCount = 0;

function talisRect(r) {
    return r && [r.x, r.y, r.width, r.height].every(function(n) {
        return typeof n === 'number' && isFinite(n);
    }) && r.width > 0 && r.height > 0;
}
function talisCovered(target, rects) {
    var pieces = [target];
    for (var i = 0; i < rects.length; ++i) {
        var r = rects[i], next = [];
        for (var j = 0; j < pieces.length; ++j) {
            var p = pieces[j], l = Math.max(p[0], r[0]), t = Math.max(p[1], r[1]);
            var right = Math.min(p[0] + p[2], r[0] + r[2]);
            var b = Math.min(p[1] + p[3], r[1] + r[3]);
            if (right <= l || b <= t) { next.push(p); continue; }
            if (t > p[1]) next.push([p[0], p[1], p[2], t - p[1]]);
            if (b < p[1] + p[3]) next.push([p[0], b, p[2], p[1] + p[3] - b]);
            if (l > p[0]) next.push([p[0], t, l - p[0], b - t]);
            if (right < p[0] + p[2]) next.push([right, t, p[0] + p[2] - right, b - t]);
        }
        if (next.length > 512) return false;
        pieces = next;
        if (!pieces.length) return true;
    }
    return false;
}
function talisOnWorkspace(w) {
    var current = workspace.currentDesktop, onDesktop = false;
    if (typeof current === 'number') {
        // Older Qt/KWin cannot marshal QVector<VirtualDesktop*>; its numeric
        // desktop and explicit all-desktops flag are still introspectable.
        onDesktop = w.onAllDesktops === true || w.desktop === current;
    } else {
        var ds = w.desktops;
        if (!ds || typeof ds.length !== 'number') return false;
        onDesktop = ds.length === 0;
        for (var i = 0; i < ds.length; ++i)
            if (ds[i].id === current.id) onDesktop = true;
    }
    var activities = w.activities;
    if (!activities || typeof activities.length !== 'number') return false;
    return onDesktop && (activities.length === 0 || activities.indexOf(workspace.currentActivity) >= 0);
}
function talisDecision(input) {
    var result = {covered: false, reason: 'uncertain'};
    if (!input || !input.enabled || input.showing_desktop !== false) {
        talisCount = 0; talisPrevious = null;
        result.reason = 'disabled-or-show-desktop'; return result;
    }
    try {
        var windows = workspace.stackingOrder;
        // 5.27 exposes the same per-window stack index via clientList(). Take
        // a bounded copy; never modify a compositor-owned list or any window.
        if (!windows && typeof workspace.clientList === 'function') {
            windows = Array.prototype.slice.call(workspace.clientList());
            if (windows.length > 128 || !windows.every(function(w) {
                return typeof w.stackingOrder === 'number' && isFinite(w.stackingOrder);
            })) throw new Error('unknown stacking order');
            windows.sort(function(a, b) { return a.stackingOrder - b.stackingOrder; });
            for (var z = 1; z < windows.length; ++z)
                if (windows[z].stackingOrder === windows[z - 1].stackingOrder) throw new Error('ambiguous stack');
        }
        if (!windows || windows.length > 128) throw new Error('bounded window list required');
        var index = -1;
        for (var i = 0; i < windows.length; ++i) {
            if (windows[i].pid === input.pid && windows[i].desktopWindow === true && windows[i].deleted === false) {
                if (index !== -1) throw new Error('ambiguous owned scene');
                index = i;
            }
        }
        if (index === -1) throw new Error('owned scene unavailable');
        var source = windows[index], geometry = source.frameGeometry;
        if (!talisRect(geometry) || source.minimized !== false || source.hidden !== false || !talisOnWorkspace(source)
            || Math.abs(geometry.width - input.width) > 1 || Math.abs(geometry.height - input.height) > 1)
            throw new Error('source geometry/workspace unavailable');
        var a = input.animation;
        if (!a || a.length !== 4 || !a.every(function(n) { return typeof n === 'number' && isFinite(n); })
            || a[0] < 0 || a[1] < 0 || a[2] <= 0 || a[3] <= 0
            || a[0] + a[2] > input.width || a[1] + a[3] > input.height)
            throw new Error('invalid animation target');
        var target = [geometry.x + a[0], geometry.y + a[1], a[2], a[3]], rects = [];
        for (i = index + 1; i < windows.length; ++i) {
            var w = windows[i];
            if (w.desktopWindow !== false || w.deleted !== false || w.minimized !== false || w.hidden !== false
                || w.shade === true || w.move === true || w.resize === true || w.opacity !== 1
                || !(w.normalWindow === true || w.dialog === true) || !talisOnWorkspace(w)) continue;
            var frame = w.frameGeometry, buffer = w.bufferGeometry, client = w.clientGeometry || frame;
            if (!talisRect(frame) || !talisRect(buffer) || !talisRect(client)) continue;
            // Only geometric interiors count: no shadows, decorations or edge sampling.
            // Client alpha is deliberately not checked, matching the chosen coverage policy.
            var l = Math.max(frame.x, buffer.x, client.x) + 2;
            var t = Math.max(frame.y, buffer.y, client.y) + 2;
            var r = Math.min(frame.x + frame.width, buffer.x + buffer.width, client.x + client.width) - 2;
            var b = Math.min(frame.y + frame.height, buffer.y + buffer.height, client.y + client.height) - 2;
            if (r > l && b > t) rects.push([l, t, r - l, b - t]);
        }
        var key = JSON.stringify([input.generation, input.width, input.height, a, geometry.x, geometry.y,
            typeof workspace.currentDesktop === 'number' ? workspace.currentDesktop : workspace.currentDesktop.id,
            workspace.currentActivity, rects]);
        var complete = talisCovered(target, rects);
        talisCount = complete ? (key === talisPrevious ? talisCount + 1 : 1) : 0;
        talisPrevious = key;
        result.covered = talisCount >= 2;
        result.reason = result.covered ? 'covered' : complete ? 'confirming' : 'exposed';
    } catch (error) {
        talisPrevious = null; talisCount = 0;
    }
    return result;
}
function talisPoll() {
    callDBus(talisService, talisPath, talisInterface, 'Snapshot', function(reply) {
        if (typeof reply !== 'string') return; // App/bus disappeared: chain stops.
        var input;
        try { input = JSON.parse(reply); } catch (error) { return; }
        if (!input) { talisPrevious = null; talisCount = 0; talisPoll(); return; }
        var decision = talisDecision(input);
        callDBus(talisService, talisPath, talisInterface, 'Observe', JSON.stringify({
            ticket: input.ticket, covered: decision.covered, reason: decision.reason
        }), function(accepted) {
            // A layout changed while this sample was in flight: reject its
            // evidence but continue with a fresh snapshot, never get stuck.
            if (typeof accepted === 'boolean') talisPoll();
        });
    });
}
talisPoll();
