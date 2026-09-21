export function createSharedSlot(initial) {
    const state = { ...initial };
    const listeners = new Set();
    function get() { return state; }
    function set(updates) { Object.assign(state, updates); for (const fn of listeners) fn(); }
    function subscribe(fn) { listeners.add(fn); return () => listeners.delete(fn); }
    return { get, set, subscribe };
}
