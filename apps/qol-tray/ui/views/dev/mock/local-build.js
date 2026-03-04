import { BUILD_ANIMATION } from '../build-animation.js';

export async function runLocalMockPluginBuild({
    getPluginIds,
    isCurrentRun,
    setBuildStarted,
    setPluginQueued,
    setPluginBuilding,
    setPluginCompleted,
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
    const completionStepDelayMs =
        BUILD_ANIMATION.completionRampMs
        + BUILD_ANIMATION.completionHoldMs
        + BUILD_ANIMATION.completionVisibleMs
        + 40;

    const pluginIds = getPluginIds()
        .filter(Boolean)
        .sort((a, b) => a.localeCompare(b));

    onClearQueuedBuildSync();
    setBuildStarted();
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
        setPluginQueued(pluginId);
        onQueueBuildSync(pluginId);
        await sleep(80);
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
        await sleep(completionStepDelayMs);
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
    return true;
}

function defaultSleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}
