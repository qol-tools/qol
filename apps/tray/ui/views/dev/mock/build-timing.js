import { BUILD_ANIMATION } from '../../../fx/build/animation.js';

const COMPILE_PHASE_COUNT = 24;
const COMPILE_STEP_COUNT = 66;
const MIN_BUILD_MS = 1000;
const MAX_BUILD_MS = 3000;

export function compileTiming() {
    const totalMs = MIN_BUILD_MS + Math.random() * (MAX_BUILD_MS - MIN_BUILD_MS);
    return {
        phaseCount: COMPILE_PHASE_COUNT,
        stepCount: COMPILE_STEP_COUNT,
        stepDelayMs: Math.round(totalMs / COMPILE_STEP_COUNT),
        completionDelayMs:
            BUILD_ANIMATION.completionRampMs
            + BUILD_ANIMATION.completionHoldMs
            + BUILD_ANIMATION.completionVisibleMs
            + 40
    };
}

export function compileStepProgress(step, timing) {
    const percent = (step * 100) / timing.stepCount;
    const phase = Math.max(1, Math.round((step * timing.phaseCount) / timing.stepCount));
    return { percent, label: `${phase}/24 compiling` };
}

export function buildResults(pluginIds) {
    return pluginIds.map(plugin_id => ({
        plugin_id,
        success: true,
        output: 'Local mock build completed',
        skipped: false
    }));
}
