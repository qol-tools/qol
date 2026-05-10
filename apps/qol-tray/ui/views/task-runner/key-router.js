import { useCallback } from 'preact/hooks';
import { useListKeyboard } from '../../lib/hooks/useListKeyboard.js';
import { useModalKeyboard } from '../../lib/hooks/useModalKeyboard.js';
import { diveViaSelector } from '../../lib/world-navigation-singleton.js';

const TASK_RUNNER_EDITOR_DIVE_SELECTOR = '[data-view-id="task-runner"]';

export function useTaskKeyHandler(data, edit) {
    const modalNav = useModalKeyboard({
        onSave: edit.saveAction,
        onClose: edit.close,
    });

    const onAdd = useCallback(() => {
        edit.openEditModal();
        diveViaSelector(TASK_RUNNER_EDITOR_DIVE_SELECTOR);
    }, [edit.openEditModal]);

    const listHandler = useListKeyboard({
        itemCount: data.actionIds.length,
        selectedIndex: data.selectedIndex,
        onAdd,
        onDelete: data.deleteAction,
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
