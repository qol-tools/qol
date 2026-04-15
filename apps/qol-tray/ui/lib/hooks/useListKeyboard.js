import { useCallback } from 'preact/hooks';

const ACTION_KEYS = {
    Enter: 'edit',
    a: 'add',
    A: 'add',
    Delete: 'delete',
    Backspace: 'delete',
};

export function useListKeyboard({ itemCount, selectedIndex, onAdd, onDelete, onEdit }) {
    return useCallback((e) => {
        const action = ACTION_KEYS[e.key];
        if (!action) return;

        e.preventDefault();

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
    }, [itemCount, selectedIndex, onAdd, onDelete, onEdit]);
}
