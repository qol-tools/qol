export async function runLocalMockPluginBuild({
    getPluginIds,
    isCurrentRun,
    setBuildStarted,
    setPluginQueued,
    setPluginBuilding,
    setPluginCompleted,
    clearPluginProgress,
    setBuildCompleted,
    onRender,
    onQueueBuildSync,
    onClearQueuedBuildSync,
    sleep = defaultSleep
}) {
    const compilePhaseCount = 24;
    const compileStepCount = 66;
    const compileTotalMs = 1320;
    const compileStepDelayMs = Math.round(compileTotalMs / compileStepCount);

    const pluginIds = getPluginIds()
        .filter(Boolean)
        .sort((a, b) => a.localeCompare(b));

    onClearQueuedBuildSync();
    setBuildStarted();

    for (const pluginId of pluginIds) {
        setPluginQueued(pluginId);
    }
    onRender();

    if (pluginIds.length === 0) {
        await sleep(100);
        if (!isCurrentRun()) return false;
        await Promise.resolve(setBuildCompleted([]));
        onRender();
        return true;
    }

    for (const pluginId of pluginIds) {
        if (!isCurrentRun()) return false;
        setPluginBuilding(pluginId, 0, '0/24 preparing');
        onQueueBuildSync(pluginId);
        await sleep(120);

        for (let step = 1; step <= compileStepCount; step += 1) {
            if (!isCurrentRun()) return false;
            const percent = (step * 100) / compileStepCount;
            const phase = Math.max(1, Math.round((step * compilePhaseCount) / compileStepCount));
            setPluginBuilding(pluginId, percent, `${phase}/24 compiling`);
            onQueueBuildSync(pluginId);
            await sleep(compileStepDelayMs);
        }

        setPluginCompleted(pluginId);
        onQueueBuildSync(pluginId);
        await sleep(220);
        clearPluginProgress(pluginId);
    }

    if (!isCurrentRun()) return false;

    await Promise.resolve(setBuildCompleted(
        pluginIds.map(plugin_id => ({
            plugin_id,
            success: true,
            output: 'Local mock build completed',
            skipped: false
        }))
    ));
    onClearQueuedBuildSync();
    return true;
}

function defaultSleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}
