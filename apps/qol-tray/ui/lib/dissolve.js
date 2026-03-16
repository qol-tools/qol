import {
    createEvaporateState, activateBatch, drawSolidPixels, processBubbles,
    evaporateFrame, runDissolve, computeCanvasSize,
    cancelExistingDissolve, createDissolveCanvas, sampleCompositeBg,
} from './dissolve-engine.js';
import { resolveColor } from './canvas.js';
import { createField, renderField } from './glitch-squares.js';

export { createEvaporateState, evaporateFrame, runDissolve };

export const DISSOLVE_PRESETS = {
    default: {
        renderScale: 2, density: 1.0, tileSize: 32,
        dissolveRate: 0.14, bubbleFade: 0.05, maxBatchRate: 0.04,
        origin: 'random',
    },
    drillDown: {
        renderScale: 2, density: 1.0, tileSize: 32,
        dissolveRate: 0.06, bubbleFade: 0.04, maxBatchRate: 0.04, ticksPerFrame: 2,
        origin: 'center', invert: true,
        speedMin: 0.02, speedRange: 0.08, wind: 0.01, wobbleAmp: 0, wobbleFreq: 0,
        colorDrift: -0.6, echoes: 3, sizeGrowth: 14, spread: 0.3, dissolveAccel: -0.8, phiAccelAt: 0.9,
    },
    goBack: {
        renderScale: 2, density: 1.0, tileSize: 32,
        dissolveRate: 0.1, bubbleFade: 0.06, maxBatchRate: 0.06,
        origin: 'edges',
        speedMin: 0.02, speedRange: 0.08, wind: 0.01, wobbleAmp: 0, wobbleFreq: 0, vortexCount: 0,
        colorDrift: 0.6, echoes: 3, sizeGrowth: 14, spread: 0.3, dissolveAccel: -0.6, phiAccelAt: 0.9, materialize: 16,
    },
    variantSwitch: {
        bleed: 20, edgeFade: 20, renderScale: 2, density: 1.0,
        tileSize: 128, dissolveRate: 0.12, bubbleFade: 0.065,
        maxBatchRate: 0.1, origin: 'center',
    },
};

export function dissolveIn(container, opts = {}) {
    cancelExistingDissolve(container);
    const canvas = createDissolveCanvas(container, opts);
    const bg = sampleCompositeBg(container);
    runDissolve(canvas, bg, () => canvas.remove(), opts.targetColor ?? 'var(--accent)', {
        width: container.offsetWidth,
        height: container.offsetHeight,
        ...opts,
    });
    return () => {
        if (canvas._dissolveCancel) canvas._dissolveCancel();
        canvas.remove();
    };
}

export function materializeInScatter(container, opts = {}) {
    if (container._materializeCancel) container._materializeCancel();
    cancelExistingDissolve(container);
    const canvas = createDissolveCanvas(container, {});
    const bg = sampleCompositeBg(container);
    const mergedOpts = {
        origin: 'random', density: 0.12, renderScale: 2,
        dissolveRate: 0.08, bubbleFade: 0.05, maxBatchRate: 0.06,
        speedMin: 0.3, speedRange: 0.8, wind: 0.1, wobbleAmp: 0, wobbleFreq: 0,
        vortexCount: 0, convergence: 0,
        width: container.offsetWidth, height: container.offsetHeight,
        materialize: 16,
        ...opts,
    };
    const s = createEvaporateState(canvas, bg, opts.targetColor ?? '#4a9eff', mergedOpts);
    const glowCanvas = document.createElement('canvas');
    glowCanvas.width = s.W;
    glowCanvas.height = s.H;
    glowCanvas.className = 'dissolve-glow';
    glowCanvas.style.cssText = canvas.style.cssText;
    glowCanvas.style.filter = 'blur(1px)';
    glowCanvas.style.zIndex = '3';
    glowCanvas.style.imageRendering = 'auto';
    container.appendChild(glowCanvas);
    const glowCtx = glowCanvas.getContext('2d');
    const glowFill = `rgb(${s.tr},${s.tg},${s.tb})`;
    let rafId = null;
    let skipCount = 0;
    s._glitchFrame = false;
    function tick() {
        skipCount++;
        const threshold = 1 + ((Math.random() * 3) | 0);
        s._glitchFrame = skipCount >= threshold;
        if (s._glitchFrame) skipCount = 0;
        const progress = s.total > 0 ? s.cursor / s.total : 0;
        const ticks = Math.max(1, Math.ceil(4 * progress * progress));
        for (let t = 0; t < ticks; t++) activateBatch(s);
        s.d32.fill(0);
        drawSolidPixels(s);
        processBubbles(s);
        s.ctx.putImageData(s.imgData, 0, 0);
        glowCtx.clearRect(0, 0, s.W, s.H);
        if (s._accentRects && s._accentRects.length) {
            glowCtx.fillStyle = glowFill;
            for (let i = 0; i < s._accentRects.length; i += 4) {
                glowCtx.fillRect(s._accentRects[i], s._accentRects[i + 1], s._accentRects[i + 2], s._accentRects[i + 3]);
            }
        }
        s.frame++;
        if (s.cursor >= s.total) {
            canvas.remove();
            glowCanvas.remove();
            container._materializeCancel = null;
            return;
        }
        rafId = requestAnimationFrame(tick);
    }
    container._materializeCancel = () => {
        if (rafId) cancelAnimationFrame(rafId);
        canvas.remove();
        glowCanvas.remove();
        container._materializeCancel = null;
    };
    rafId = requestAnimationFrame(tick);
}

export function materializeIn(container, opts = {}) {
    if (container._materializeCancel) container._materializeCancel();
    cancelExistingDissolve(container);
    const canvas = createDissolveCanvas(container, {});
    const bg = sampleCompositeBg(container);
    const mergedOpts = {
        origin: 'random', density: 1.0, renderScale: 2,
        dissolveRate: 0.12, bubbleFade: 0.05, maxBatchRate: 0.08,
        speedMin: 0.3, speedRange: 0.8, wind: 0.1, wobbleAmp: 0, wobbleFreq: 0,
        vortexCount: 0, convergence: 0,
        width: container.offsetWidth, height: container.offsetHeight,
        materialize: 0,
        ...opts,
    };
    const s = createEvaporateState(canvas, bg, opts.targetColor ?? '#4a9eff', mergedOpts);
    const glowCanvas = document.createElement('canvas');
    glowCanvas.width = s.W;
    glowCanvas.height = s.H;
    glowCanvas.className = 'dissolve-glow';
    glowCanvas.style.cssText = canvas.style.cssText;
    glowCanvas.style.filter = 'blur(1px)';
    glowCanvas.style.zIndex = '3';
    glowCanvas.style.imageRendering = 'auto';
    container.appendChild(glowCanvas);
    const glowCtx = glowCanvas.getContext('2d');
    const field = createField(s.W, s.H, `rgb(${s.tr},${s.tg},${s.tb})`, `rgb(${s.r},${s.g},${s.b})`);
    let rafId = null;
    function tick() {
        activateBatch(s);
        const progress = s.total > 0 ? s.cursor / s.total : 0;
        if (progress > 0.5) activateBatch(s);
        if (progress > 0.8) activateBatch(s);
        s.d32.fill(0);
        drawSolidPixels(s);
        s.ctx.putImageData(s.imgData, 0, 0);
        glowCtx.clearRect(0, 0, s.W, s.H);
        renderField(s.ctx, glowCtx, field, progress);
        if (s.cursor >= s.total) {
            canvas.remove();
            glowCanvas.remove();
            container._materializeCancel = null;
            return;
        }
        rafId = requestAnimationFrame(tick);
    }
    container._materializeCancel = () => {
        if (rafId) cancelAnimationFrame(rafId);
        canvas.remove();
        glowCanvas.remove();
        container._materializeCancel = null;
    };
    rafId = requestAnimationFrame(tick);
}
