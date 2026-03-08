function syncIdleRow(rowRef, completion, clearOverlayNodes, fill) {
    if (completion.syncPlayback(rowRef)) return true;
    if (completion.startIfReady(rowRef)) return true;
    clearOverlayNodes(rowRef, fill.stopFillAnimation);
    return true;
}

function syncCompletedRow(rowRef, completion, ensureRowOverlayNodes) {
    if (completion.syncPlayback(rowRef)) return true;
    if (!ensureRowOverlayNodes(rowRef)) return false;
    if (!completion.start(rowRef, true)) return false;
    return completion.syncPlayback(rowRef);
}

function syncActiveRow(rowRef, buildState, buildAnimation, normalizePercent, formatDetail, ensureRowOverlayNodes, completion, fill, setOverlayCopy) {
    if (completion.getState(rowRef.pluginId) === 'playing') return true;
    if (!ensureRowOverlayNodes(rowRef)) return false;
    const label = buildState.status === 'queued' ? 'Queued' : 'Compiling';
    const detail = formatDetail(buildState.phase, buildState.percent);
    const normalizedPercent = normalizePercent(buildState.percent);
    const cappedPercent = buildState.status === 'building'
        ? Math.min(normalizedPercent, buildAnimation.completionTriggerPercent - 0.2)
        : normalizedPercent;
    completion.clear(rowRef.pluginId);
    rowRef.completing = false;
    rowRef.overlay.classList.remove('is-completing');
    fill.resetStaleProgressState(rowRef, buildState.status, normalizedPercent);
    const displayPercent = fill.toDisplayPercent(rowRef, cappedPercent, buildState.status);
    fill.setFillTarget(rowRef, displayPercent, buildState.status !== 'building');
    setOverlayCopy(rowRef, label, detail);
    return true;
}

function doSyncRow(pluginId, rowRefs, getContainer, getPluginById, getBuildState, syncIdle, syncCompleted, syncActive) {
    if (!getContainer()) return false;
    const rowRef = rowRefs.get(pluginId);
    if (!rowRef) return false;
    const plugin = getPluginById(pluginId);
    if (!plugin) return false;
    const buildState = getBuildState(plugin);
    rowRef.row.classList.toggle('is-building', !!buildState);
    if (!buildState) return syncIdle(rowRef);
    if (buildState.status === 'completed') return syncCompleted(rowRef);
    return syncActive(rowRef, buildState);
}

export function createRowSync({
    buildAnimation, rowRefs, getContainer, getPluginById, getBuildState,
    formatDetail, normalizePercent, ensureRowOverlayNodes, clearOverlayNodes, setOverlayCopy, completion, fill
}) {
    const syncIdle = rowRef => syncIdleRow(rowRef, completion, clearOverlayNodes, fill);
    const syncCompleted = rowRef => syncCompletedRow(rowRef, completion, ensureRowOverlayNodes);
    const syncActive = (rowRef, bs) => syncActiveRow(rowRef, bs, buildAnimation, normalizePercent, formatDetail, ensureRowOverlayNodes, completion, fill, setOverlayCopy);
    return { syncRow: pluginId => doSyncRow(pluginId, rowRefs, getContainer, getPluginById, getBuildState, syncIdle, syncCompleted, syncActive) };
}
