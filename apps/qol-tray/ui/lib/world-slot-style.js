// Pure helpers for world slot DOM styling. Extracted from WorldViewSlot in
// ui/app/views.js so the slot-style invariants can be locked down by tests
// without importing the full view module graph.
//
// The "slot is the content" model: when entry.contentSized is true, the slot
// emits no inline height — natural content height drives the slot box, and
// the world camera follows selection across tall pages. When false, the slot
// pins to entry.height (legacy fixed-page behavior, still used for editor
// sub-pages).

export function isSlotVisible(entry, cameraLayer, confinedPages, diveDepth) {
    const layerMatch = entry.layer === cameraLayer;
    const confined = confinedPages && confinedPages.length > 0;
    const ascending = entry.layer < 0 && (diveDepth ?? 0) === 0;
    return layerMatch && !ascending && (!confined || confinedPages.includes(entry.id));
}

export function slotStyle(entry, visible) {
    const heightStyle = entry.contentSized ? '' : ` height:${entry.height}px;`;
    return `left:${entry.x}px; top:${entry.y}px; width:${entry.width}px;${heightStyle}${visible ? '' : ' display:none;'}`;
}
