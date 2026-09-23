export function shuffle(len) {
    const arr = Array.from({ length: len }, (_, i) => i);
    for (let i = len - 1; i > 0; i--) {
        const j = Math.floor(Math.random() * (i + 1));
        [arr[i], arr[j]] = [arr[j], arr[i]];
    }
    return arr;
}

export function resolveColor(cssValue) {
    const el = document.createElement('div');
    el.style.cssText = `display:none;color:${cssValue}`;
    document.body.appendChild(el);
    const rgb = getComputedStyle(el).color;
    document.body.removeChild(el);
    const m = rgb.match(/\d+/g);
    return m ? [+m[0], +m[1], +m[2]] : [26, 30, 38];
}

export function sizeToParent(canvas) {
    const { offsetWidth: w, offsetHeight: h } = canvas.parentElement;
    canvas.width = w;
    canvas.height = h;
    return [w, h];
}

export function filledImageData(ctx, w, h, r, g, b) {
    const imgData = ctx.createImageData(w, h);
    const d = imgData.data;
    for (let i = 0; i < w * h; i++) {
        d[i * 4] = r;
        d[i * 4 + 1] = g;
        d[i * 4 + 2] = b;
        d[i * 4 + 3] = 255;
    }
    return imgData;
}
