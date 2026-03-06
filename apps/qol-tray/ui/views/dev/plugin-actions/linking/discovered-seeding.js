export function seedDiscoveredFromLinked(state, pluginId) {
    if (!pluginId) {
        return;
    }

    const linked = state.plugins.find(plugin => plugin.id === pluginId);
    const merged = state.mergedList.find(plugin => plugin.id === pluginId);
    const path = linked?.source || merged?.path || '';
    if (!path) {
        return;
    }

    const seeded = {
        id: pluginId,
        name: linked?.name || merged?.name || pluginId,
        path
    };

    const existingIndex = state.discovered.findIndex(plugin => plugin.id === pluginId);
    if (existingIndex >= 0) {
        state.discovered[existingIndex] = {
            ...state.discovered[existingIndex],
            ...seeded
        };
        return;
    }

    state.discovered.push(seeded);
}
