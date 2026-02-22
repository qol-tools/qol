function cloneTargets(targets) {
    return new Set(targets);
}

function withModel(model, patch) {
    return {
        activeRunId: model.activeRunId,
        source: model.source,
        activeTargets: cloneTargets(model.activeTargets),
        ...patch
    };
}

export function createMockFlowModel() {
    return {
        activeRunId: 0,
        source: null,
        activeTargets: new Set()
    };
}

export function beginMockFlowRun(model) {
    const nextRunId = model.activeRunId + 1;
    const nextModel = withModel(model, {
        activeRunId: nextRunId,
        source: null,
        activeTargets: new Set()
    });
    return {
        model: nextModel,
        runId: nextRunId,
        mockTesting: true
    };
}

export function stopMockFlowRun(model) {
    const nextModel = withModel(model, {
        activeRunId: model.activeRunId + 1,
        source: null,
        activeTargets: new Set()
    });
    return {
        model: nextModel,
        mockTesting: false
    };
}

export function isCurrentMockRun(model, runId, mockTesting) {
    return !!mockTesting && model.activeRunId === runId;
}

export function setMockFlowSource(model, source) {
    return withModel(model, { source });
}

export function getMockFlowSource(model) {
    return model.source;
}

export function setBackendTargets(model, targetIds) {
    const activeTargets = new Set();
    for (const targetId of targetIds || []) {
        if (typeof targetId === 'string' && targetId.length) {
            activeTargets.add(targetId);
        }
    }
    const nextTesting = activeTargets.size > 0;
    return {
        model: withModel(model, {
            source: nextTesting ? 'backend' : null,
            activeTargets
        }),
        mockTesting: nextTesting
    };
}

export function hasActiveBackendTargets(model) {
    return model.source === 'backend' && model.activeTargets.size > 0;
}

export function completeBackendTarget(model, targetId, mockTesting) {
    if (!mockTesting || model.source !== 'backend') {
        return { model, mockTesting, completed: false };
    }
    if (!model.activeTargets.has(targetId)) {
        return { model, mockTesting, completed: false };
    }
    const nextTargets = cloneTargets(model.activeTargets);
    nextTargets.delete(targetId);
    const done = nextTargets.size === 0;
    return {
        model: withModel(model, {
            source: done ? null : 'backend',
            activeTargets: nextTargets
        }),
        mockTesting: done ? false : mockTesting,
        completed: done
    };
}

export function pruneInactiveTargets(model, runningById) {
    const nextTargets = cloneTargets(model.activeTargets);
    let changed = false;
    for (const targetId of model.activeTargets) {
        if (runningById.get(targetId) !== true) {
            nextTargets.delete(targetId);
            changed = true;
        }
    }
    const done = nextTargets.size === 0;
    return {
        model: withModel(model, {
            source: done ? null : model.source,
            activeTargets: nextTargets
        }),
        changed,
        done
    };
}

export function finishMockTesting(model) {
    return {
        model: withModel(model, {
            source: null,
            activeTargets: new Set()
        }),
        mockTesting: false
    };
}
