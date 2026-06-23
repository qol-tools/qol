import { useCallback, useMemo } from 'preact/hooks';
import { useListKeyboard } from './useListKeyboard.js';
import { useModalKeyboard } from './useModalKeyboard.js';
import { composeListEditorHandler } from './list-editor-dispatch.js';

export function useListEditorKeyboard({
    editModalRef,
    onModalSave,
    onModalClose,
    itemCount,
    selectedIndex,
    onAdd,
    onDelete,
    preIntercept,
    listIntercept,
} = {}) {
    const modalNav = useModalKeyboard({ onSave: onModalSave, onClose: onModalClose });
    const listHandler = useListKeyboard({ itemCount, selectedIndex, onAdd, onDelete });
    const handleKey = useMemo(
        () => composeListEditorHandler({
            modalRef: editModalRef,
            onModal: modalNav.handleKey,
            onList: listHandler,
            preIntercept,
            listIntercept,
        }),
        [editModalRef, modalNav.handleKey, listHandler, preIntercept, listIntercept],
    );
    const isBlocking = useCallback(() => editModalRef.current !== null, [editModalRef]);
    return { handleKey, isBlocking, modalNav };
}
