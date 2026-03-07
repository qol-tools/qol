import { useCallback } from 'preact/hooks';
import { withShiftVariants, dispatchKey } from '../../utils/keys.js';

function routeTaskKey(e, data, edit, test) {
    if (edit.editModalRef.current) {
        if (e.key === 'Escape') { e.preventDefault(); edit.close(); return; }
        if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); edit.saveAction(); }
        return;
    }
    if (test.testingIdRef.current) {
        if (e.key === 'Escape') { e.preventDefault(); test.closeTestPanel(); }
        return;
    }
    routeNormalKey(e, data, edit);
}

function routeNormalKey(e, data, edit) {
    const ids = data.actionIdsRef.current;
    const idx = data.selectedIndexRef.current;
    dispatchKey(e, withShiftVariants({
        ArrowUp: () => data.setSelectedIndex(i => Math.max(0, i - 1)),
        ArrowDown: () => data.setSelectedIndex(i => Math.min(ids.length - 1, i + 1)),
        Enter: () => { if (ids.length > 0) edit.openEditModal(ids[idx]); },
    }));
}

export function useTaskKeyHandler(data, edit, test) {
    const handleKey = useCallback(
        e => routeTaskKey(e, data, edit, test),
        [edit.saveAction, test.closeTestPanel, edit.openEditModal, test.openTestPanel, data.deleteAction, data.copyApiExample]
    );
    const isBlocking = useCallback(
        () => edit.editModalRef.current !== null || test.testingIdRef.current !== null,
        []
    );
    return { handleKey, isBlocking };
}
