import { useCallback } from 'preact/hooks';
import { useStateRef } from '../../hooks/useStateRef.js';
import { buildSavedActions, createEditModalState, persistTaskRunnerConfig } from './data.js';

function doSave(actionsRef, editModal, setActions, setActionIds, setSelectedIndex, setEditModal) {
    if (!editModal) return;
    const saved = buildSavedActions(actionsRef.current, editModal);
    if (!saved.actionId || !saved.actions[saved.actionId]?.name || !saved.actions[saved.actionId]?.command) return;
    const nextIds = Object.keys(saved.actions);
    setActions(saved.actions);
    setActionIds(nextIds);
    if (editModal.isNew) setSelectedIndex(nextIds.length - 1);
    void persistTaskRunnerConfig(saved.actions);
    setEditModal(null);
}

export function useEditModal(actionsRef, setActions, setActionIds, setSelectedIndex) {
    const [editModal, setEditModal, editModalRef] = useStateRef(null);
    const openEditModal = useCallback(
        (actionId = null) => setEditModal(createEditModalState(actionsRef.current, actionId)),
        []
    );
    const saveAction = useCallback(
        () => doSave(actionsRef, editModal, setActions, setActionIds, setSelectedIndex, setEditModal),
        [editModal]
    );
    const updateField = useCallback(
        (field, value) => setEditModal(prev => prev ? { ...prev, [field]: value } : prev),
        []
    );
    const close = useCallback(() => setEditModal(null), []);
    return { editModal, editModalRef, openEditModal, saveAction, updateField, close };
}
