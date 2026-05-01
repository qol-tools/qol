const DOT_SPACING = 50;
const DOT_SIZE = 1;
const DOT_ALPHA_BASE = 0.03;
const MIN_SCREEN_SPACING = 4;

export function createWorldCanvasBg(canvas, camera) {
    let ctx = canvas.getContext('2d');
    let mounted = true;

    function draw() {
        if (!mounted) return;
        const dpr = window.devicePixelRatio || 1;
        const w = canvas.clientWidth;
        const h = canvas.clientHeight;
        if (w === 0 || h === 0) return;
        if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
            canvas.width = w * dpr;
            canvas.height = h * dpr;
            ctx = canvas.getContext('2d');
        }
        try {
            ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
            ctx.clearRect(0, 0, w, h);
        } catch {
            canvas.width = w * dpr;
            canvas.height = h * dpr;
            ctx = canvas.getContext('2d');
            return;
        }

        const z = camera.zoom;
        const spacing = DOT_SPACING * z;
        if (spacing < MIN_SCREEN_SPACING) return;

        const alpha = DOT_ALPHA_BASE * Math.min(1, spacing / 20);
        ctx.fillStyle = `rgba(255, 255, 255, ${alpha})`;

        const offsetX = (((-camera.x % DOT_SPACING) + DOT_SPACING) % DOT_SPACING) * z;
        const offsetY = (((-camera.y % DOT_SPACING) + DOT_SPACING) % DOT_SPACING) * z;

        for (let x = offsetX; x < w; x += spacing) {
            for (let y = offsetY; y < h; y += spacing) {
                ctx.fillRect(x, y, DOT_SIZE, DOT_SIZE);
            }
        }
    }

    const unsub = camera.subscribe(() => draw());
    const ro = new ResizeObserver(() => draw());
    ro.observe(canvas);
    draw();

    return {
        destroy() {
            mounted = false;
            unsub();
            ro.disconnect();
        },
    };
}
