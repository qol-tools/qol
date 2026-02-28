const listeners = new Set();
const reconnectListeners = new Set();
let eventSource = null;
let connected = false;
let suspended = false;

export function subscribe(callback) {
    listeners.add(callback);
    if (!suspended) ensureConnected();
    return () => listeners.delete(callback);
}

export function onReconnect(callback) {
    reconnectListeners.add(callback);
    return () => reconnectListeners.delete(callback);
}

// Pause SSE when the window loses focus — frees daemon from sending events
// to a client that isn't processing them. Resume on focus.
export function suspend() {
    if (suspended) return;
    suspended = true;
    if (eventSource) {
        eventSource.close();
        eventSource = null;
    }
}

export function resume() {
    if (!suspended) return;
    suspended = false;
    if (listeners.size > 0) ensureConnected();
}

function ensureConnected() {
    if (eventSource || suspended) return;

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
        if (!suspended) setTimeout(ensureConnected, 1000);
    };
}
