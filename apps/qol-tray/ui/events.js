const listeners = new Set();
let eventSource = null;

export function subscribe(callback) {
    listeners.add(callback);
    ensureConnected();
    return () => listeners.delete(callback);
}

function ensureConnected() {
    if (eventSource) return;

    eventSource = new EventSource('/api/events');
    eventSource.onmessage = (e) => {
        for (const listener of listeners) {
            listener(e.data);
        }
    };
    eventSource.onerror = () => {
        eventSource?.close();
        eventSource = null;
        setTimeout(ensureConnected, 1000);
    };
}
