export const BROWSER_RESERVED_CHORDS = Object.freeze([
    { key: 'r', mods: ['meta'] },
    { key: 'r', mods: ['meta', 'shift'] },
    { key: 'F5', mods: [] },
    { key: 'F5', mods: ['meta'] },
    { key: 'F5', mods: ['shift'] },
    { key: 'F12', mods: [] },
    { key: 'i', mods: ['meta', 'shift'] },
    { key: 'j', mods: ['meta', 'shift'] },
    { key: 'c', mods: ['meta', 'shift'] },
    { key: '0', mods: ['meta'] },
    { key: '=', mods: ['meta'] },
    { key: '+', mods: ['meta'] },
    { key: '+', mods: ['meta', 'shift'] },
    { key: '-', mods: ['meta'] },
    { key: 'f', mods: ['meta'] },
    { key: 'w', mods: ['meta'] },
    { key: 'p', mods: ['meta'] },
].map(spec => Object.freeze({ ...spec, mods: Object.freeze(spec.mods) })));

export function isReservedChord(event) {
    return BROWSER_RESERVED_CHORDS.some(spec => matchesChord(event, spec));
}

function matchesChord(event, spec) {
    const requireMeta = spec.mods.includes('meta');
    const requireShift = spec.mods.includes('shift');
    const requireAlt = spec.mods.includes('alt');
    if (Boolean(event.ctrlKey || event.metaKey) !== requireMeta) return false;
    if (Boolean(event.shiftKey) !== requireShift) return false;
    if (Boolean(event.altKey) !== requireAlt) return false;
    const eventKey = typeof event.key === 'string' ? event.key.toLowerCase() : '';
    return eventKey === spec.key.toLowerCase();
}

let bypassDepth = 0;

export function pauseKeyguard() { bypassDepth++; }
export function resumeKeyguard() { if (bypassDepth > 0) bypassDepth--; }
export function isKeyguardActive() { return bypassDepth === 0; }

export function createKeyguardPause() {
    let paused = false;
    return {
        pause() {
            if (paused) return;
            paused = true;
            pauseKeyguard();
        },
        resume() {
            if (!paused) return;
            paused = false;
            resumeKeyguard();
        },
    };
}

function guard(event) {
    if (!isKeyguardActive()) return;
    if (isReservedChord(event)) event.stopImmediatePropagation();
}

if (typeof window !== 'undefined') window.addEventListener('keydown', guard, true);
if (typeof document !== 'undefined') document.addEventListener('keydown', guard, true);
