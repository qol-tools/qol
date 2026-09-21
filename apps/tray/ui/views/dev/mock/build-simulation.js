import { compileTiming, compileStepProgress } from './build-timing.js';

export async function* simulatePluginBuild(pluginId, sleep) {
    const timing = compileTiming();
    const variance = 0.6 + Math.random() * 0.8;
    const vary = ms => Math.round(ms * variance);

    yield { type: 'queued', pluginId };
    await sleep(vary(80));
    yield { type: 'building', pluginId, percent: 0, label: '0/24 preparing' };
    await sleep(vary(120));

    for (let step = 1; step <= timing.stepCount; step += 1) {
        const progress = compileStepProgress(step, timing);
        yield { type: 'building', pluginId, percent: progress.percent, label: progress.label };
        await sleep(vary(timing.stepDelayMs));
    }

    yield { type: 'completed', pluginId };
    await sleep(timing.completionDelayMs);
}
