const NAV_BINDINGS = new Map([
    ['Tab', { action: (a, e) => a.navigate(e.shiftKey ? -1 : 1) }],
    ['ArrowDown', { action: (a) => a.navigate(1) }],
    ['j', { action: (a) => a.navigate(1) }],
    ['ArrowUp', { action: (a) => a.navigate(-1) }],
    ['k', { action: (a) => a.navigate(-1) }],
    ['ArrowRight', { action: (a) => a.focusDetail() }],
    ['l', { action: (a) => a.focusDetail() }],
    ['Enter', { action: (a) => a.focusDetail() }],
]);

const DETAIL_BINDINGS = new Map([
    ['Escape', { action: (a) => a.focusNav() }],
    ['ArrowLeft', { action: (a) => a.focusNav(), skipEditable: true }],
    ['h', { action: (a) => a.focusNav(), skipEditable: true }],
]);

export function installInput(zones, actions) {
    document.addEventListener('keydown', (e) => {
        const zone = resolveZone(zones);
        if (!zone) return;
        const bindings = zone === 'nav' ? NAV_BINDINGS : DETAIL_BINDINGS;
        const entry = bindings.get(e.key);
        if (!entry) return;
        if (entry.skipEditable && isEditable(document.activeElement)) return;
        e.preventDefault();
        entry.action(actions, e);
    });
}

function resolveZone(zones) {
    const active = document.activeElement;
    if (zones.nav()?.contains(active)) return 'nav';
    if (zones.detail()?.contains(active)) return 'detail';
    return null;
}

function isEditable(el) {
    const tag = el?.tagName;
    return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT';
}
