import { BUILD_ANIMATION } from '../build-animation.js';
import { applyFillScale, clearOverlayNodes, ensureOverlayNodes, finiteOr, setOverlayCopy } from './dom.js';
import { createCompletionController } from './completion.js';
import { createFillController } from './fill.js';
import { createRowSync } from './sync.js';

export function createOverlayCtx({ getContainer, getPluginById, getBuildState, formatDetail, normalizePercent }) {
    const rowRefs = new Map();
    const pendingBuildRows = new Set();
    const ensureRowOverlayNodes = rowRef => ensureOverlayNodes(rowRef, pct => applyFillScale(rowRef, pct, normalizePercent));
    const fill = createFillController({ buildAnimation: BUILD_ANIMATION, normalizePercent, applyFillScale, finiteOr });
    const completion = createCompletionController({
        buildAnimation: BUILD_ANIMATION, rowRefs, normalizePercent,
        ensureOverlayNodes: ensureRowOverlayNodes, clearOverlayNodes,
        stopFillAnimation: fill.stopFillAnimation, applyFillScale, setOverlayCopy, finiteOr
    });
    const rowSync = createRowSync({
        buildAnimation: BUILD_ANIMATION, rowRefs, getContainer, getPluginById, getBuildState,
        formatDetail, normalizePercent, ensureRowOverlayNodes, clearOverlayNodes, setOverlayCopy, completion, fill
    });
    return { rowRefs, pendingBuildRows, buildSyncFrame: null, fill, completion, rowSync, getContainer };
}
