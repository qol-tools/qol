import { useCallback } from 'preact/hooks';

const STANDARD_KEYS = {
    ArrowUp: 'up',
    ArrowDown: 'down',
    k: 'up',
    K: 'up',
    j: 'down',
    J: 'down',
    Enter: 'edit',
    a: 'add',
    A: 'add',
    Delete: 'delete',
    Backspace: 'delete',
};

export function useListKeyboard({ surfaceSelector, itemCount, selectedIndex, setSelectedIndex, onAdd, onDelete, onEdit }) {
    return useCallback((e) => {
        const action = STANDARD_KEYS[e.key];
        if (!action) return;

        e.preventDefault();

        if (action === 'up') {
            const next = Math.max(0, selectedIndex - 1);
            setSelectedIndex(next);
            focusSurface(surfaceSelector, next);
            return;
        }
        if (action === 'down') {
            const next = Math.min(itemCount - 1, selectedIndex + 1);
            setSelectedIndex(next);
            focusSurface(surfaceSelector, next);
            return;
        }
        if (action === 'add' && onAdd) {
            onAdd();
            return;
        }
        if (action === 'delete' && onDelete) {
            onDelete();
            return;
        }
        if (action === 'edit' && onEdit && itemCount > 0 && selectedIndex >= 0) {
            onEdit();
            return;
        }
    }, [surfaceSelector, itemCount, selectedIndex, setSelectedIndex, onAdd, onDelete, onEdit]);
}

function focusSurface(selector, index) {
    if (!selector) return;
    const el = document.querySelector(`${selector}[data-index="${index}"]`);
    if (el) el.focus();
}
