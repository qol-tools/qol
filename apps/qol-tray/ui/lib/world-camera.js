export function createCamera() {
    let x = 0;
    let y = 0;
    let worldEl = null;
    let animId = 0;
    let animFrom = null;
    let animTarget = null;
    let animStart = 0;
    let animDuration = 0;
    const listeners = new Set();

    function notify() {
        for (const fn of listeners) fn(x, y);
    }

    function apply() {
        if (worldEl) worldEl.style.transform = `translate(${-x}px, ${-y}px)`;
        notify();
    }

    function panTo(nx, ny) {
        cancelSmooth();
        x = nx;
        y = ny;
        apply();
    }

    function panSmooth(tx, ty, duration) {
        cancelSmooth();
        animFrom = { x, y };
        animTarget = { x: tx, y: ty };
        animStart = performance.now();
        animDuration = duration;
        animId = requestAnimationFrame(tick);
    }

    function cancelSmooth() {
        if (animId) { cancelAnimationFrame(animId); animId = 0; }
        animTarget = null;
    }

    function tick(now) {
        if (!animTarget) return;
        const t = Math.min(1, (now - animStart) / animDuration);
        const e = 1 - Math.pow(1 - t, 3);
        x = animFrom.x + (animTarget.x - animFrom.x) * e;
        y = animFrom.y + (animTarget.y - animFrom.y) * e;
        apply();
        if (t < 1) {
            animId = requestAnimationFrame(tick);
        } else {
            animTarget = null;
            animId = 0;
        }
    }

    function nudge(dx, dy) {
        cancelSmooth();
        x += dx;
        y += dy;
        apply();
    }

    return {
        get x() { return x; },
        get y() { return y; },
        get animating() { return animTarget !== null; },
        setWorldElement(el) { worldEl = el; },
        panTo,
        panSmooth,
        cancelSmooth,
        nudge,
        subscribe(fn) { listeners.add(fn); return () => listeners.delete(fn); },
    };
}
