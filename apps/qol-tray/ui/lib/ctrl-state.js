let held = false;
const listeners = new Set();

function onKeyDown(e) {
    if (e.key === 'Control' && !held) {
        held = true;
        document.body.dataset.ctrlHeld = '';
        notify();
    }
}

function onKeyUp(e) {
    if (e.key === 'Control' && held) {
        held = false;
        delete document.body.dataset.ctrlHeld;
        notify();
    }
}

function notify() {
    for (const fn of listeners) fn(held);
}

function onBlur() {
    if (!held) return;
    held = false;
    delete document.body.dataset.ctrlHeld;
    notify();
}

document.addEventListener('keydown', onKeyDown, true);
document.addEventListener('keyup', onKeyUp, true);
window.addEventListener('blur', onBlur);

export function isCtrlHeld() { return held; }

export function subscribeCtrl(fn) {
    listeners.add(fn);
    return () => listeners.delete(fn);
}
