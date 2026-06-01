import { fetchMockTargetsState } from './mock/api.js';
import { mockTargetForEvent } from './mock/event-map.js';
import {
    beginNewRun,
    completeMockTarget,
    handleLegacyStart,
    handleNewApiStart,
    stopMockFlows
} from './mock/flow-executor.js';
import { createMockFlowModel, getMockFlowSource, setBackendTargets } from './mock/reducer.js';

export function createMockController({ state, buildController, getMergedPlugins, onNeedsRender }) {
    const ctx = { state, buildController, getMergedPlugins, onNeedsRender, model: createMockFlowModel() };
    return {
        handleEvent: event => dispatchMockEvent(ctx, event),
        hydrateMockTargets: (skip = false) => hydrateMockTargets(ctx, skip),
        triggerMockFlows: () => triggerMockFlows(ctx),
        isMockTesting: () => ctx.state.mockTesting,
        completeMockTarget: targetId => completeMockTarget(ctx, targetId)
    };
}

function dispatchMockEvent(ctx, event) {
    const targetId = mockTargetForEvent(event.type);
    if (targetId) completeMockTarget(ctx, targetId);
}

async function hydrateMockTargets(ctx, skipUpdate) {
    if (getMockFlowSource(ctx.model) === 'local') return;
    const targetState = await fetchMockTargetsState();
    if (!targetState) return;
    const next = setBackendTargets(ctx.model, targetState.runningIds);
    ctx.model = next.model;
    ctx.state.mockTesting = next.mockTesting;
    if (!skipUpdate && !ctx.state.linkingId) ctx.onNeedsRender();
}

async function triggerMockFlows(ctx) {
    if (ctx.state.mockTesting) { await stopMockFlows(ctx); return; }
    const runId = beginNewRun(ctx);
    const outcome = await handleNewApiStart(ctx, runId);
    if (outcome !== 'legacy') return;
    await handleLegacyStart(ctx, runId);
}
