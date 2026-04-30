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
