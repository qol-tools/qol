export function createHydrationScheduler() {
    let timerId = null;

    function clear() {
        if (timerId === null) return;
        clearTimeout(timerId);
        timerId = null;
    }

    function schedule(state, pluginId, hydrate, attempts = 6) {
        if (!state.cpuMonitoring[pluginId] || state.cpuByPlugin[pluginId]) return;
        if (attempts <= 0) return;
        clear();
        timerId = setTimeout(() => {
            timerId = null;
            if (!state.cpuMonitoring[pluginId]) return;
            void hydrate().then(() => schedule(state, pluginId, hydrate, attempts - 1));
        }, 1000);
    }

    return { clear, schedule };
}
