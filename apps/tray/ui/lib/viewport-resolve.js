const VIEWPORT_ID = 'viewport';

export function resolveViewport(viewportRef, doc = (typeof document !== 'undefined' ? document : null)) {
    const cached = viewportRef?.current;
    if (cached && cached.isConnected && cached.clientWidth > 0) return cached;
    const fresh = (doc && typeof doc.getElementById === 'function')
        ? doc.getElementById(VIEWPORT_ID)
        : null;
    if (fresh && viewportRef) viewportRef.current = fresh;
    return fresh;
}
