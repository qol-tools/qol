import { BUILD_ANIMATION } from '../build-animation.js';

const COMPILE_PHASE_COUNT = 24;
const COMPILE_STEP_COUNT = 66;
const COMPILE_TOTAL_MS = 1320;

export function compileTiming() {
    return {
        phaseCount: COMPILE_PHASE_COUNT,
        stepCount: COMPILE_STEP_COUNT,
        stepDelayMs: Math.round(COMPILE_TOTAL_MS / COMPILE_STEP_COUNT),
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
