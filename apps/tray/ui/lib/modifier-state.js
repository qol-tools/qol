let snapshot = { ctrl: false, shift: false, alt: false, meta: false };
const listeners = new Set();

const FIELD_BY_KEY = { Control: 'ctrl', Shift: 'shift', Alt: 'alt', Meta: 'meta' };

function commit(next) {
    if (
        next.ctrl === snapshot.ctrl &&
        next.shift === snapshot.shift &&
        next.alt === snapshot.alt &&
        next.meta === snapshot.meta
    ) return;
    snapshot = next;
    renderBodyAttrs(next);
    for (const listener of listeners) listener();
}

function setHeld(field, held) {
    if (snapshot[field] === held) return;
    commit({ ...snapshot, [field]: held });
}

function renderBodyAttrs(state) {
    if (typeof document === 'undefined' || !document.body) return;
    const body = document.body;
    if (state.ctrl) body.dataset.ctrlHeld = '';
    else delete body.dataset.ctrlHeld;
    if (state.shift) body.setAttribute('data-shift-held', '');
    else body.removeAttribute('data-shift-held');
}

function onKeyDown(event) {
    const field = FIELD_BY_KEY[event.key];
    if (field) setHeld(field, true);
}

function onKeyUp(event) {
    const field = FIELD_BY_KEY[event.key];
    if (field) setHeld(field, false);
}

function onBlur() {
    commit({ ctrl: false, shift: false, alt: false, meta: false });
}

if (typeof window !== 'undefined') {
    window.addEventListener('keydown', onKeyDown, true);
    window.addEventListener('keyup', onKeyUp, true);
    window.addEventListener('blur', onBlur);
}

export function getModifierState() {
    return snapshot;
}

export function subscribeModifiers(listener) {
    listeners.add(listener);
    return () => listeners.delete(listener);
}

export function isCtrlHeld() {
    return snapshot.ctrl;
}

export function subscribeCtrl(listener) {
    let last = snapshot.ctrl;
    return subscribeModifiers(() => {
        if (snapshot.ctrl === last) return;
        last = snapshot.ctrl;
        listener(snapshot.ctrl);
    });
}
