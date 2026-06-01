const installing = new Map();
const listeners = new Set();

export function add(id, name) {
    installing.set(id, { id, name });
    notify();
}

export function remove(id) {
    installing.delete(id);
    notify();
}

export function has(id) {
    return installing.has(id);
}

export function getAll() {
    return Array.from(installing.values());
}

export function subscribe(callback) {
    listeners.add(callback);
    return () => listeners.delete(callback);
}

function notify() {
    for (const listener of listeners) {
        listener(getAll());
    }
}
