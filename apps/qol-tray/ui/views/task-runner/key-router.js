import { useCallback } from 'preact/hooks';
import { useListEditorKeyboard } from '../../lib/hooks/useListEditorKeyboard.js';
import { diveViaSelector } from '../../lib/world-navigation-singleton.js';

const TASK_RUNNER_EDITOR_DIVE_SELECTOR = '[data-view-id="task-runner"]';

export function useTaskKeyHandler(data, edit) {
    const onAdd = useCallback(() => {
        edit.openEditModal();
        diveViaSelector(TASK_RUNNER_EDITOR_DIVE_SELECTOR);
    }, [edit.openEditModal]);

    return useListEditorKeyboard({
        editModalRef: edit.editModalRef,
        onModalSave: edit.saveAction,
        onModalClose: edit.close,
        itemCount: data.actionIds.length,
        selectedIndex: data.selectedIndex,
        onAdd,
        onDelete: data.deleteAction,
    });
}
