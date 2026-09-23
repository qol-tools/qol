import { buildLegacyStartErrorMessage, stopMockTargetsApi } from './api.js';
import { runLocalMockPluginBuild, defaultSleep } from './local-build.js';
import { runLegacyFallback, tryStartMockTargets } from './trigger-flows.js';
import {
    beginMockFlowRun,
    completeBackendTarget,
    finishMockTesting,
    getMockFlowSource,
    isCurrentMockRun,
    setBackendTargets,
    stopMockFlowRun
} from './reducer.js';
import { MOCK_FLOWS_DONE } from './events.js';

export function beginNewRun(ctx) {
    const started = beginMockFlowRun(ctx.model);
    ctx.model = started.model;
    ctx.state.mockTesting = started.mockTesting;
    ctx.state.error = null;
    ctx.onNeedsRender();
    return started.runId;
}

function isCurrent(ctx, runId) {
    return isCurrentMockRun(ctx.model, runId, ctx.state.mockTesting);
}

function finishTesting(ctx) {
    const next = finishMockTesting(ctx.model);
    ctx.model = next.model;
    ctx.state.mockTesting = next.mockTesting;
}

function failRun(ctx, message) {
    ctx.state.error = message;
    finishTesting(ctx);
    ctx.onNeedsRender();
}

function applyNewApiTargets(ctx, targets) {
    const result = setBackendTargets(ctx.model, targets);
    ctx.model = result.model;
    ctx.state.mockTesting = result.mockTesting;
    ctx.onNeedsRender();
}

export async function handleNewApiStart(ctx, runId) {
    const result = await tryStartMockTargets();
    if (!isCurrent(ctx, runId)) return 'cancelled';
    if (result.targets) { applyNewApiTargets(ctx, result.targets); return 'done'; }
    if (result.error) { failRun(ctx, result.error); return 'done'; }
    return 'legacy';
}

export async function handleLegacyStart(ctx, runId) {
    const legacy = await runLegacyFallback(ctx.model);
    if (!isCurrent(ctx, runId)) return;
    ctx.model = legacy.model;
    if (legacy.needsLocalFallback) {
        const ok = await executeLocalBuild(ctx, runId);
        if (!ok || !isCurrent(ctx, runId)) return;
        finishTesting(ctx);
    }
    applyLegacyBuildResult(ctx, legacy);
    await applyLegacyErrors(ctx, legacy);
    ctx.onNeedsRender();
}

function applyLegacyBuildResult(ctx, legacy) {
    if (legacy.needsLocalFallback || !legacy.buildRes?.ok) return;
    const result = setBackendTargets(ctx.model, ['plugin_build']);
    ctx.model = result.model;
    ctx.state.mockTesting = result.mockTesting;
}

async function applyLegacyErrors(ctx, legacy) {
    const failure = await buildLegacyStartErrorMessage(
        legacy.updateRes, legacy.recompileRes, legacy.buildRes, legacy.needsLocalFallback
    );
    if (failure.message) ctx.state.error = failure.message;
    if (failure.buildFailed) finishTesting(ctx);
}

async function executeLocalBuild(ctx, runId) {
    const pluginIds = ctx.getMergedPlugins()
        .map(p => p.id)
        .filter(Boolean)
        .sort((a, b) => a.localeCompare(b));
    ctx.buildController.clearQueuedBuildRowSync();
    ctx.state.building = true;
    ctx.state.buildResults = null;
    ctx.state.buildProgress = {};
    ctx.onNeedsRender();
    return runLocalMockPluginBuild(
        pluginIds,
        () => isCurrent(ctx, runId),
        step => applyBuildStep(ctx, step),
        results => ctx.buildController.completeLocalBuild(results),
        defaultSleep
    );
}

function applyBuildStep(ctx, step) {
    const progress = STEP_TO_PROGRESS[step.type]?.(step);
    if (progress) ctx.state.buildProgress[step.pluginId] = progress;
    ctx.buildController.queueBuildRowSync(step.pluginId);
}

const STEP_TO_PROGRESS = {
    queued: () => ({ status: 'queued', percent: 0, phase: 'Queued' }),
    building: s => ({ status: 'building', percent: s.percent, phase: s.label }),
    completed: () => ({ status: 'completed', percent: 100, phase: 'Completed' })
};

export async function stopMockFlows(ctx) {
    if (!ctx.state.mockTesting) return;
    const source = getMockFlowSource(ctx.model);
    const stopped = stopMockFlowRun(ctx.model);
    ctx.model = stopped.model;
    ctx.state.mockTesting = stopped.mockTesting;
    if (source === 'local') ctx.buildController.stopLocalMockBuildUi();
    ctx.onNeedsRender();
    if (source === 'backend') await stopMockTargetsApi();
}

export function completeMockTarget(ctx, targetId) {
    const result = completeBackendTarget(ctx.model, targetId, ctx.state.mockTesting);
    ctx.model = result.model;
    ctx.state.mockTesting = result.mockTesting;
    if (!result.completed) return;
    ctx.buildController.stopLocalMockBuildUi();
    ctx.onNeedsRender();
    document.dispatchEvent(new CustomEvent(MOCK_FLOWS_DONE));
}
