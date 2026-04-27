// Resolve the world viewport DOM element, recovering when the cached ref is
// stale. During dive transitions the `#viewport` element can be replaced; the
// shared ref in App.js then points at a detached node (clientWidth=0,
// isConnected=false), which collapses every viewport-derived calculation
// (minimap rect, camera follow, focus tracking) to zero until the next ref
// write.
//
// The recovery is two-step:
//   1. Trust the cache when it's connected and has non-zero width.
//   2. Otherwise re-query the DOM by id and overwrite the cache so other
//      consumers (camera.getViewportSize, navigation.domHelpers) recover too.
//
// Pure helper so it can be unit-tested without Preact / DOM bootstrapping —
// callers pass any object with a mutable `current` property and the doc.

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
