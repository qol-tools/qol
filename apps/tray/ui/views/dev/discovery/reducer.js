export function nextDiscoveryStartedState() {
    return { discovering: true };
}

export function nextDiscoveryCompletedState(plugins) {
    return {
        discovering: false,
        discovered: plugins || []
    };
}

export function parseDiscoveryPayload(payload, currentDiscovered = []) {
    const status = typeof payload?.status === 'string' ? payload.status : 'idle';
    return {
        discovering: status === 'discovering',
        discovered: status === 'complete' && Array.isArray(payload?.plugins)
            ? payload.plugins
            : currentDiscovered
    };
}

export function parseLogControlsPayload(payload) {
    if (payload && typeof payload === 'object') {
        return payload;
    }
    return {};
}
