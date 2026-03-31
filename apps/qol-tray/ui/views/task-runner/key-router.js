import { useCallback } from 'preact/hooks';
import { useListKeyboard } from '../../hooks/useListKeyboard.js';
import { useModalKeyboard } from '../../hooks/useModalKeyboard.js';
export function useTaskKeyHandler(data, edit, test) {
    const modalNav = useModalKeyboard({
        onSave: edit.saveAction,
        onClose: edit.close,
    });

    const listHandler = useListKeyboard({
        surfaceSelector: '.actions-list [data-selected-surface]',
        itemCount: data.actionIds.length,
        selectedIndex: data.selectedIndex,
        setSelectedIndex: data.setSelectedIndex,
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
        if (test.testingIdRef.current) {
            if (e.key === 'Escape') { e.preventDefault(); test.closeTestPanel(); }
            return;
        }
        listHandler(e);
    }, [listHandler, modalNav.handleKey, test.closeTestPanel]);

    const isBlocking = useCallback(
        () => edit.editModalRef.current !== null || test.testingIdRef.current !== null,
        []
    );
    return { handleKey, isBlocking, modalNav };
}
