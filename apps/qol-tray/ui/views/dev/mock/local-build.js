export async function runLocalMockPluginBuild({
    getPluginIds,
    isCurrentRun,
    setBuildStarted,
    setPluginQueued,
    setPluginBuilding,
    setBuildCompleted,
    onRender,
    onQueueBuildSync,
    onClearQueuedBuildSync,
    sleep = defaultSleep
}) {
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
        setBuildCompleted([]);
        onRender();
        return true;
    }

    for (const pluginId of pluginIds) {
        if (!isCurrentRun()) return false;
        setPluginBuilding(pluginId, 0, '0/24 preparing');
        onQueueBuildSync(pluginId);
        await sleep(120);

        for (let done = 1; done <= 24; done += 1) {
            if (!isCurrentRun()) return false;
            setPluginBuilding(pluginId, Math.floor((done * 100) / 24), `${done}/24 compiling`);
            onQueueBuildSync(pluginId);
            await sleep(55);
        }
    }

    if (!isCurrentRun()) return false;

    setBuildCompleted(
        pluginIds.map(plugin_id => ({
            plugin_id,
            success: true,
            output: 'Local mock build completed',
            skipped: false
        }))
    );
    onClearQueuedBuildSync();
    onRender();
    return true;
}

function defaultSleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}
