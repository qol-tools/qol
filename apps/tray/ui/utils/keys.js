export function withShiftVariants(handlers) {
    const result = {};
    for (const [key, fn] of Object.entries(handlers)) {
        result[key] = fn;
        if (key.length === 1) result[key === key.toLowerCase() ? key.toUpperCase() : key.toLowerCase()] = fn;
    }
    return result;
}

export function dispatchKey(e, handlers) {
    const handler = handlers[e.key];
    if (!handler) return;
    e.preventDefault();
    handler();
}
