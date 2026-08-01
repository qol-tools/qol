import {
    createEvaporateState,
    evaporateFrame, runDissolve, computeCanvasSize,
    cancelExistingDissolve, createDissolveCanvas, sampleCompositeBg,
} from './engine.js';
import { resolveColor } from '../canvas.js';

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
