const RETARGET_INTERVAL = 5000;
const SPEED = 0.04;

function rand(min, max) {
    return min + Math.random() * (max - min);
}

function newTarget() {
    return {
        x1: rand(5, 45), y1: rand(-25, 15),
        x2: rand(55, 95), y2: rand(-20, 20),
    };
}

export function randomizeGlow(el) {
    if (!el || el._glowActive) return;
    el._glowActive = true;

    const pos = newTarget();
    let target = newTarget();
    apply(el, pos);

    setInterval(() => { target = newTarget(); }, RETARGET_INTERVAL);

    function tick() {
        pos.x1 += (target.x1 - pos.x1) * SPEED;
        pos.y1 += (target.y1 - pos.y1) * SPEED;
        pos.x2 += (target.x2 - pos.x2) * SPEED;
        pos.y2 += (target.y2 - pos.y2) * SPEED;
        apply(el, pos);
        requestAnimationFrame(tick);
    }

    requestAnimationFrame(tick);
}

function apply(el, s) {
    el.style.setProperty('--glow-x1', `${s.x1}%`);
    el.style.setProperty('--glow-y1', `${s.y1}%`);
    el.style.setProperty('--glow-x2', `${s.x2}%`);
    el.style.setProperty('--glow-y2', `${s.y2}%`);
}
