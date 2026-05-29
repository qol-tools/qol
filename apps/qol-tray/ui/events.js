const listeners = new Set();
const reconnectListeners = new Set();
let eventSource = null;
let connected = false;

const RETRY_BASE_MS = 1000;
const RETRY_MAX_MS = 30000;
let retryDelay = RETRY_BASE_MS;

export function subscribe(callback) {
    listeners.add(callback);
    ensureConnected();
    return () => listeners.delete(callback);
}

export function onReconnect(callback) {
    reconnectListeners.add(callback);
    return () => reconnectListeners.delete(callback);
}

function ensureConnected() {
    if (eventSource) return;

    eventSource = new EventSource('/api/events');
    eventSource.onopen = () => {
        retryDelay = RETRY_BASE_MS;
        if (connected) {
            for (const cb of reconnectListeners) cb();
        }
        connected = true;
    };
    eventSource.onmessage = (e) => {
        let event;
        try {
            event = JSON.parse(e.data);
        } catch {
            return;
        }
        for (const listener of listeners) {
            listener(event);
        }
    };
    eventSource.onerror = () => {
        eventSource?.close();
        eventSource = null;
        const jitter = Math.random() * 0.3 * retryDelay;
        setTimeout(ensureConnected, retryDelay + jitter);
        retryDelay = Math.min(retryDelay * 2, RETRY_MAX_MS);
    };
}
