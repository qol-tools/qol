import { jsonRequest, tryFetchJson } from '../../../api/client.js';

export function createSyncChain() {
    let chain = Promise.resolve();

    function queue(pluginIds) {
        chain = chain.catch(() => {}).then(() => pushMonitoringState(pluginIds));
        return chain;
    }

    return { queue };
}

async function pushMonitoringState(pluginIds) {
    try {
        await fetch('/api/dev/plugin-cpu/monitoring', {
            ...jsonRequest('PUT', { plugin_ids: pluginIds })
        });
    } catch {}
}

export async function fetchCpuSnapshot() {
    return tryFetchJson('/api/dev/plugin-cpu');
}
