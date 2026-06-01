import { buildResults } from './build-timing.js';
import { simulatePluginBuild } from './build-simulation.js';

export async function runLocalMockPluginBuild(
    pluginIds, isCurrentRun, applyStep, completeBuild, sleep
) {
    if (pluginIds.length === 0) {
        await sleep(100);
        if (!isCurrentRun()) return false;
        await Promise.resolve(completeBuild([]));
        return true;
    }

    const runs = pluginIds.map(async pluginId => {
        for await (const step of simulatePluginBuild(pluginId, sleep)) {
            if (!isCurrentRun()) return;
            applyStep(step);
        }
    });
    await Promise.all(runs);

    if (!isCurrentRun()) return false;
    await Promise.resolve(completeBuild(buildResults(pluginIds)));
    return true;
}

export function defaultSleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}
