export function createCompletionStore({ createSnapshot, computeRemainingMs, finiteOr }) {
    const completionByPlugin = new Map();

    function clear(pluginId) {
        if (!pluginId) {
            return;
        }

        completionByPlugin.delete(pluginId);
    }

    function clearAll() {
        completionByPlugin.clear();
    }

    function entries() {
        return completionByPlugin.entries();
    }

    function get(pluginId) {
        return completionByPlugin.get(pluginId) || null;
    }

    function getState(pluginId) {
        return get(pluginId)?.state || 'idle';
    }

    function remainingMs(pluginId, now) {
        const completion = get(pluginId);
        if (!completion || completion.state !== 'playing') {
            return 0;
        }

        return computeRemainingMs(completion, now);
    }

    function snapshot(pluginId, now) {
        const completion = get(pluginId);
        if (!completion || completion.state !== 'playing') {
            return null;
        }

        return createSnapshot(completion, now);
    }

    function setState(pluginId, state, patch = {}) {
        if (!pluginId) {
            return;
        }

        if (state === 'idle') {
            completionByPlugin.delete(pluginId);
            return;
        }

        const previous = completionByPlugin.get(pluginId) || {};
        completionByPlugin.set(pluginId, {
            state,
            startedAt: finiteOr(patch.startedAt, finiteOr(previous.startedAt, 0)),
            startPercent: finiteOr(patch.startPercent, finiteOr(previous.startPercent, 100)),
            phase: typeof patch.phase === 'string' ? patch.phase : (previous.phase || 'ramp'),
            phaseStartedAt: finiteOr(
                patch.phaseStartedAt,
                finiteOr(previous.phaseStartedAt, finiteOr(patch.startedAt, finiteOr(previous.startedAt, 0)))
            )
        });
    }

    function finalize(pluginId) {
        setState(pluginId, 'done');
    }

    return {
        clear,
        clearAll,
        entries,
        finalize,
        get,
        getState,
        remainingMs,
        setState,
        snapshot
    };
}
