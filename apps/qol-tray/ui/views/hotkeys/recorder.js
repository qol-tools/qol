const MODIFIER_KEYS = ['Control', 'Alt', 'Shift', 'Meta'];
const MODIFIER_NAMES = ['Ctrl', 'Alt', 'Shift', 'Super'];

const NAV_KEY_MAP = {
    Space: 'Space', Enter: 'Enter', Escape: 'Escape', Tab: 'Tab',
    Backspace: 'Backspace', Delete: 'Delete', Insert: 'Insert',
    Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown',
    ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right',
    F1: 'F1', F2: 'F2', F3: 'F3', F4: 'F4', F5: 'F5', F6: 'F6',
    F7: 'F7', F8: 'F8', F9: 'F9', F10: 'F10', F11: 'F11', F12: 'F12',
    PrintScreen: 'PrintScreen', Pause: 'Pause'
};

export function applyRecordingKey(modal, event) {
    if (event.key === 'Escape') {
        return { modal: { ...modal, recording: false }, advance: false };
    }
    if (MODIFIER_KEYS.includes(event.key)) {
        const key = formatKeyEvent(event);
        return { modal: key ? { ...modal, key } : modal, advance: false };
    }

    const key = formatKeyEvent(event);
    if (!key || MODIFIER_NAMES.includes(key)) {
        return { modal, advance: false };
    }

    return {
        modal: { ...modal, key, recording: false },
        advance: true
    };
}

function formatKeyEvent(event) {
    const parts = [];
    if (event.ctrlKey) parts.push('Ctrl');
    if (event.altKey) parts.push('Alt');
    if (event.shiftKey) parts.push('Shift');
    if (event.metaKey) parts.push('Super');
    if (MODIFIER_KEYS.includes(event.key)) {
        return parts.join('+') || '';
    }

    const key = getKeyName(event.code);
    if (key) parts.push(key);
    return parts.join('+');
}

function getKeyName(code) {
    if (code.startsWith('Key')) return code.slice(3);
    if (code.startsWith('Digit')) return code.slice(5);
    if (code.startsWith('Numpad')) return code;
    return NAV_KEY_MAP[code] || null;
}
