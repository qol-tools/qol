const listeners = new Set();
const reconnectListeners = new Set();
let eventSource = null;
let connected = false;

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
        setTimeout(ensureConnected, 1000);
    };
}
