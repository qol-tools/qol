export function applyNavigateRoute(
    route,
    loc = typeof window !== 'undefined' ? window.location : undefined,
    win = typeof window !== 'undefined' ? window : undefined,
) {
    if (typeof route !== 'string' || !loc) return false;
    const r = route.trim().replace(/^#/, '').replace(/^\//, '');
    if (!r) return false;
    const target = '#' + r;
    if (loc.hash === target) {
        // Assigning an identical hash does not fire `hashchange`, so the router
        // would never re-resolve the deep link. Dispatch one so a repeat
        // navigate to the route already in the address bar still re-resolves.
        dispatchHashChange(win);
        return true;
    }
    loc.hash = target;
    return true;
}

function dispatchHashChange(win) {
    if (!win || typeof win.dispatchEvent !== 'function') return;
    const Ctor = typeof win.HashChangeEvent === 'function'
        ? win.HashChangeEvent
        : (typeof win.Event === 'function' ? win.Event : null);
    if (Ctor) win.dispatchEvent(new Ctor('hashchange'));
}
