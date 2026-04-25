import { useCallback } from 'preact/hooks';
import { useListKeyboard } from '../../lib/hooks/useListKeyboard.js';
import { useModalKeyboard } from '../../lib/hooks/useModalKeyboard.js';
export function useTaskKeyHandler(data, edit) {
    const modalNav = useModalKeyboard({
        onSave: edit.saveAction,
        onClose: edit.close,
    });

    const listHandler = useListKeyboard({
        itemCount: data.actionIds.length,
        selectedIndex: data.selectedIndex,
        onAdd: edit.openEditModal,
        onDelete: data.deleteAction,
        onEdit: useCallback(() => {
            const ids = data.actionIdsRef.current;
            const idx = data.selectedIndexRef.current;
            if (ids.length > 0) edit.openEditModal(ids[idx]);
        }, [edit.openEditModal]),
    });

    const handleKey = useCallback((e) => {
        if (edit.editModalRef.current) {
            modalNav.handleKey(e);
            return;
        }
        listHandler(e);
    }, [listHandler, modalNav.handleKey]);

    const isBlocking = useCallback(
        () => edit.editModalRef.current !== null,
        []
    );
    return { handleKey, isBlocking, modalNav };
}
