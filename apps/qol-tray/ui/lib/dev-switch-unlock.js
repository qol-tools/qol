export const STORAGE_KEY = 'qol:dev-switch-revealed';
export const CLICK_THRESHOLD = 7;
export const CLICK_WINDOW_MS = 2000;
export const TYPE_SEQUENCE = 'dev';
export const TYPE_GAP_MS = 1500;

export function createUnlockTracker({ now = Date.now, storage } = {}) {
    const store = storage ?? (typeof localStorage !== 'undefined' ? localStorage : null);
    let revealed = store?.getItem(STORAGE_KEY) === '1';
    let clicks = [];
    let typed = [];
    let listener = null;

    function emit() {
        if (listener) listener(revealed);
    }

    function reveal() {
        if (revealed) return;
        clicks = [];
        typed = [];
        revealed = true;
        store?.setItem(STORAGE_KEY, '1');
        emit();
    }

    function bumpClick() {
        if (revealed) return;
        const t = now();
        clicks = clicks.filter(x => t - x < CLICK_WINDOW_MS);
        clicks.push(t);
        if (clicks.length >= CLICK_THRESHOLD) reveal();
    }

    function feedKey(key) {
        if (revealed) return;
        if (typeof key !== 'string' || key.length !== 1) return;
        const ch = key.toLowerCase();
        const t = now();
        const last = typed[typed.length - 1];
        if (last && t - last.t > TYPE_GAP_MS) typed = [];
        const expected = TYPE_SEQUENCE[typed.length];
        if (ch !== expected) {
            typed = ch === TYPE_SEQUENCE[0] ? [{ ch, t }] : [];
            return;
        }
        typed.push({ ch, t });
        if (typed.length === TYPE_SEQUENCE.length) reveal();
    }

    return {
        isRevealed: () => revealed,
        bumpClick,
        feedKey,
        subscribe: (cb) => { listener = cb; return () => { if (listener === cb) listener = null; }; },
    };
}
