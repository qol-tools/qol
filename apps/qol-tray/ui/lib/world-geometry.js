export function cameraCenterFor(entry, viewportW, viewportH, zoom) {
    const z = zoom || 1;
    return {
        x: entry.x + entry.width / 2 - viewportW / (2 * z),
        y: entry.y + entry.height / 2 - viewportH / (2 * z),
    };
}

export function withPadding(rect, padX, padY) {
    return {
        x: rect.x - padX,
        y: rect.y - padY,
        width: rect.width + padX * 2,
        height: rect.height + padY * 2,
        layer: rect.layer,
    };
}

export function boundsOfEntries(entries) {
    if (!entries.length) return null;
    let x0 = Infinity;
    let y0 = Infinity;
    let x1 = -Infinity;
    let y1 = -Infinity;
    for (const e of entries) {
        if (e.x < x0) x0 = e.x;
        if (e.y < y0) y0 = e.y;
        if (e.x + e.width > x1) x1 = e.x + e.width;
        if (e.y + e.height > y1) y1 = e.y + e.height;
    }
    return { x: x0, y: y0, width: x1 - x0, height: y1 - y0 };
}

export function maxEntryExtent(entries) {
    let maxW = 0;
    let maxH = 0;
    for (const e of entries) {
        if (e.width > maxW) maxW = e.width;
        if (e.height > maxH) maxH = e.height;
    }
    return { padX: maxW, padY: maxH };
}
