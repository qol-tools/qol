export function hueComponents(h) {
    const x = 1 - Math.abs(((h / 60) % 2) - 1);
    if (h < 60)  return [1, x, 0];
    if (h < 120) return [x, 1, 0];
    if (h < 180) return [0, 1, x];
    if (h < 240) return [0, x, 1];
    if (h < 300) return [x, 0, 1];
    return [1, 0, x];
}

export function hueSatToHex(h, s) {
    const [hr, hg, hb] = hueComponents(h);
    const r = Math.round((1 - s + s * hr) * 255);
    const g = Math.round((1 - s + s * hg) * 255);
    const b = Math.round((1 - s + s * hb) * 255);
    return [r, g, b].map(c => c.toString(16).padStart(2, '0')).join('');
}

export function hexToHueSat(hex) {
    const clean = hex.replace(/^#/, '').slice(0, 6).padEnd(6, '0');
    const r = parseInt(clean.substring(0, 2), 16) / 255;
    const g = parseInt(clean.substring(2, 4), 16) / 255;
    const b = parseInt(clean.substring(4, 6), 16) / 255;
    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    const delta = max - min;
    let h = 0;
    if (delta > 0) {
        if (max === r) h = 60 * (((g - b) / delta + 6) % 6);
        else if (max === g) h = 60 * ((b - r) / delta + 2);
        else h = 60 * ((r - g) / delta + 4);
    }
    const s = max === 0 ? 0 : delta / max;
    return { hue: h, saturation: s };
}
