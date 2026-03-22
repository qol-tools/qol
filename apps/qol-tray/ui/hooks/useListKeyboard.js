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

export function useListKeyboard({ itemCount, selectedIndex, setSelectedIndex, onAdd, onDelete, onEdit }) {
    return useCallback((e) => {
        const action = STANDARD_KEYS[e.key];
        if (!action) return;

        e.preventDefault();

        if (action === 'up') {
            setSelectedIndex(i => Math.max(0, i - 1));
            return;
        }
        if (action === 'down') {
            setSelectedIndex(i => Math.min(itemCount - 1, i + 1));
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
    }, [itemCount, selectedIndex, setSelectedIndex, onAdd, onDelete, onEdit]);
}
