let pendingShortcutPrefill = null;
const subscribers = new Set();

export function setPendingShortcutPrefill(prefill) {
    pendingShortcutPrefill = prefill;
    for (const fn of subscribers) fn();
}

export function takePendingShortcutPrefill() {
    const p = pendingShortcutPrefill;
    pendingShortcutPrefill = null;
    return p;
}

export function subscribeShortcutPrefill(fn) {
    subscribers.add(fn);
    return () => subscribers.delete(fn);
}
