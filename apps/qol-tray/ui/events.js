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
