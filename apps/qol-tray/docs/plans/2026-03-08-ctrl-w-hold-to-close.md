# CTRL+W Hold-to-Close Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Intercept CTRL+W and replace browser tab-close with a 2.5s hold-to-confirm flow — fast backdrop fade-in, real-time countdown timer, and a full-viewport bubble evaporation animation timed to complete at t=0.

**Architecture:** Single self-contained `CloseGuard` component registered once in `AppShell`. Owns its keyboard listeners, rAF countdown, and canvas evaporation animation. No other component is modified for this feature. Canvas utilities (`shuffle`, `resolveColor`, `filledImageData`) are imported from `lib/canvas.js` (pure utilities, no coupling); evaporation animation logic is local to `CloseGuard.js`.

**Tech Stack:** Preact, htm, Canvas 2D API, `requestAnimationFrame`, CSS custom properties

---

### Task 1: CSS

**Files:**
- Create: `ui/styles/close-guard.css`
- Modify: `ui/styles/styles.css`

**Step 1: Create `ui/styles/close-guard.css`**

```css
.close-guard-backdrop {
    position: fixed;
    inset: 0;
    z-index: 9000;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(var(--overlay-ink-rgb), 0.82);
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.15s ease;
}

.close-guard-backdrop.visible {
    opacity: 1;
    pointer-events: all;
}

.close-guard-timer {
    position: relative;
    z-index: 9001;
    font-size: 5rem;
    font-variant-numeric: tabular-nums;
    color: var(--text-primary);
    transition: color 0.3s ease;
    user-select: none;
    letter-spacing: -0.02em;
}

.close-guard-timer.danger {
    color: var(--danger);
}

.close-guard-canvas {
    position: fixed;
    inset: 0;
    z-index: 9002;
    pointer-events: none;
}
```

**Step 2: Add import to `ui/styles/styles.css`**

Add after the last existing `@import`:
```css
@import "./close-guard.css";
```

**Step 3: Manual verify**

Open the app — no visual change expected. No regressions.

---

### Task 2: CloseGuard component

**Files:**
- Create: `ui/components/CloseGuard.js`

**Step 1: Write the full component**

```js
import { html } from '../lib/html.js';
import { useRef, useState, useEffect } from 'preact/hooks';
import { shuffle, resolveColor, filledImageData } from '../lib/canvas.js';

const HOLD_MS = 2500;
const RED_AT_MS = 1000;
const BUBBLE_AT_MS = 1300;
const DISSOLVE_RATE = 0.09;
const BUBBLE_FADE = 0.035;
const BUBBLE_SPEED_MIN = 0.35;
const BUBBLE_SPEED_RANGE = 0.9;
const BUBBLE_WOBBLE_AMP = 1.5;
const BUBBLE_WOBBLE_FREQ = 0.28;

function createEvaporateState(canvas) {
    const W = window.innerWidth;
    const H = window.innerHeight;
    canvas.width = W;
    canvas.height = H;
    const ctx = canvas.getContext('2d');
    const [r, g, b] = resolveColor('var(--bg-base)');
    const total = W * H;
    const imgData = filledImageData(ctx, W, H, r, g, b);
    ctx.putImageData(imgData, 0, 0);
    return {
        ctx, W, H, r, g, b, total, imgData,
        d: imgData.data,
        indices: shuffle(total),
        activated: new Int32Array(total).fill(-1),
        cursor: 0,
        frame: 0,
    };
}

function evaporateFrame(s) {
    const batch = Math.max(1, Math.ceil((s.total - s.cursor) * DISSOLVE_RATE));
    const end = Math.min(s.cursor + batch, s.total);
    for (let i = s.cursor; i < end; i++) s.activated[s.indices[i]] = s.frame;
    s.cursor = end;
    s.d.fill(0);
    let anyMoving = false;
    for (let i = 0; i < s.total; i++) {
        if (s.activated[i] < 0) {
            const off = i * 4;
            s.d[off] = s.r; s.d[off + 1] = s.g; s.d[off + 2] = s.b; s.d[off + 3] = 255;
            continue;
        }
        const age = s.frame - s.activated[i];
        const alpha = 1 - age * BUBBLE_FADE;
        if (alpha <= 0) continue;
        anyMoving = true;
        const spd = BUBBLE_SPEED_MIN + ((Math.imul(i, 1234567891) >>> 0) / 4294967296) * BUBBLE_SPEED_RANGE;
        const ph = ((Math.imul(i, 2654435761) >>> 0) / 4294967296) * Math.PI * 2;
        const newX = Math.min(s.W - 1, Math.max(0, Math.round(i % s.W + Math.sin(ph + age * BUBBLE_WOBBLE_FREQ) * BUBBLE_WOBBLE_AMP)));
        const newY = Math.round((i / s.W | 0) - age * spd);
        if (newY < 0 || newY >= s.H) continue;
        const noff = (newY * s.W + newX) * 4;
        const a = (alpha * 255) | 0;
        if (a <= s.d[noff + 3]) continue;
        s.d[noff] = s.r; s.d[noff + 1] = s.g; s.d[noff + 2] = s.b; s.d[noff + 3] = a;
    }
    s.ctx.putImageData(s.imgData, 0, 0);
    s.frame++;
    return s.cursor >= s.total && !anyMoving;
}

export function CloseGuard() {
    const [visible, setVisible] = useState(false);
    const [remaining, setRemaining] = useState(HOLD_MS);
    const [danger, setDanger] = useState(false);

    const phaseRef = useRef('idle');
    const startRef = useRef(null);
    const rafRef = useRef(null);
    const bubbleRafRef = useRef(null);
    const canvasRef = useRef(null);

    useEffect(() => {
        const startEvaporation = () => {
            const canvas = canvasRef.current;
            if (!canvas) return;
            const s = createEvaporateState(canvas);
            const tick = () => {
                if (evaporateFrame(s)) { bubbleRafRef.current = null; return; }
                bubbleRafRef.current = requestAnimationFrame(tick);
            };
            bubbleRafRef.current = requestAnimationFrame(tick);
        };

        const tick = () => {
            const elapsed = performance.now() - startRef.current;
            const rem = Math.max(0, HOLD_MS - elapsed);
            setRemaining(rem);
            setDanger(rem <= RED_AT_MS);
            if (rem <= 0) { window.close(); return; }
            if (rem <= BUBBLE_AT_MS && phaseRef.current === 'holding') {
                phaseRef.current = 'dissolving';
                startEvaporation();
            }
            rafRef.current = requestAnimationFrame(tick);
        };

        const cancel = () => {
            if (phaseRef.current === 'idle') return;
            phaseRef.current = 'idle';
            startRef.current = null;
            if (rafRef.current) cancelAnimationFrame(rafRef.current);
            if (bubbleRafRef.current) cancelAnimationFrame(bubbleRafRef.current);
            rafRef.current = null;
            bubbleRafRef.current = null;
            const canvas = canvasRef.current;
            if (canvas) canvas.getContext('2d').clearRect(0, 0, canvas.width, canvas.height);
            setVisible(false);
            setDanger(false);
            setRemaining(HOLD_MS);
        };

        const onKeyDown = (e) => {
            if (!e.ctrlKey || e.key !== 'w') return;
            e.preventDefault();
            if (phaseRef.current !== 'idle') return;
            phaseRef.current = 'holding';
            startRef.current = performance.now();
            setVisible(true);
            rafRef.current = requestAnimationFrame(tick);
        };

        const onKeyUp = (e) => {
            if (e.key === 'w' || e.key === 'Control') cancel();
        };

        document.addEventListener('keydown', onKeyDown);
        document.addEventListener('keyup', onKeyUp);
        return () => {
            document.removeEventListener('keydown', onKeyDown);
            document.removeEventListener('keyup', onKeyUp);
        };
    }, []);

    const timerCls = 'close-guard-timer' + (danger ? ' danger' : '');
    const backdropCls = 'close-guard-backdrop' + (visible ? ' visible' : '');

    return html`
        <div class=${backdropCls}>
            <span class=${timerCls}>${(remaining / 1000).toFixed(1)}</span>
        </div>
        <canvas ref=${canvasRef} class="close-guard-canvas" />
    `;
}
```

**Note on function length:** `evaporateFrame` and the `useEffect` body both approach 20 lines due to the pixel-loop math. If either exceeds 20 lines in the editor, extract `activateNewPixels(s)` and `drawPixel(s, i)` as helpers — only do this if the line count actually triggers the threshold.

**Note on bubble timing:** `DISSOLVE_RATE = 0.09` and `BUBBLE_FADE = 0.035` are tuned for ~1.3s at 60fps on a 1920×1080 viewport. If the animation finishes too early or too late relative to the countdown, adjust `BUBBLE_AT_MS` first (earlier = more buffer), then `DISSOLVE_RATE`.

---

### Task 3: Register in AppShell

**Files:**
- Modify: `ui/components/App.js`

**Step 1: Import and render CloseGuard**

In `App.js`, add import:
```js
import { CloseGuard } from './CloseGuard.js';
```

In `AppShell`, render `<CloseGuard />` as the last child of the root `app-container` div, so it sits above all content but is wired into the same Preact tree:

```js
return html`
    <div class="app-container">
        <div class="app-main">
            ...existing content...
        </div>
        <div class="app-footer">
            ...existing content...
        </div>
        <${CloseGuard} />
    </div>
`;
```

---

### Task 4: Manual verification

**Hold flow:**
1. Open app, press and hold CTRL+W — backdrop fades in fast, timer shows 2.5 and counts down
2. Release before 1s — backdrop fades out, app returns to normal
3. Hold past 1s — timer text turns red
4. Hold until ~1.2s remaining — bubble evaporation starts across full viewport
5. Hold full 2.5s — `window.close()` fires (tab closes or browser blocks it for non-script-opened tabs)

**Cancel on Ctrl release:** Hold CTRL+W, then release Ctrl key (not W) — should also cancel.

**No double-trigger:** Tap CTRL+W rapidly several times — no stacking.

**Bubble timing:** The evaporation should finish approximately when the countdown hits 0. Adjust constants in `CloseGuard.js` if off.

---

### Commit sequence

```
feat: add CTRL+W hold-to-close guard with backdrop and evaporation effect
```

Single commit after all tasks are verified.
