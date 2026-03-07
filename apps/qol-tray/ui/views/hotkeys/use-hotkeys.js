import { useEffect, useCallback } from 'preact/hooks';
import { useStateRef } from '../../hooks/useStateRef.js';
import { usePersistedIndex } from '../../hooks/usePersistedIndex.js';
import { withShiftVariants, dispatchKey } from '../../utils/keys.js';
import {
    buildSavedHotkeys,
    getAvailableActions as resolveAvailableActions,
    loadHotkeysViewData,
    nextSelectedIndex,
    persistHotkeys,
    removeHotkeyAtIndex
} from './data.js';
import {
    applyRecordingKey,
    changeEditModalPlugin,
    createEditModalState,
    nextEditModalState
} from './modal.js';
import { resolveModalKeyAction, applyModalAction } from './keys.js';

export function useHotkeys() {
    const d = useHotkeysData();
    const m = useModalActions(d);
    const { deleteSelected } = useListActions(d);
    const { handleKey, isBlocking } = useKeyboard(d, m, deleteSelected);
    return {
        hotkeys: d.hotkeys,
        plugins: d.plugins,
        selectedIndex: d.selectedIndex,
        editModal: d.editModal,
        setSelectedIndex: d.setSelectedIndex,
        openEditModal: m.openEditModal,
        handlePluginChange: m.handlePluginChange,
        handleActionChange: m.handleActionChange,
        startRecording: m.startRecording,
        closeModal: m.closeModal,
        saveHotkey: m.saveHotkey,
        deleteSelected,
        handleKey,
        isBlocking
    };
}

function useHotkeysData() {
    const [hotkeys, setHotkeys, hotkeysRef] = useStateRef([]);
    const [plugins, setPlugins] = useStateRef([]);
    const [selectedIndex, setSelectedIndex, selectedIndexRef, markRestored] = usePersistedIndex('hotkeys-selected-index', -1);
    const [editModal, setEditModal, editModalRef] = useStateRef(null);
    const [modalFieldIndex, setModalFieldIndex, modalFieldIndexRef] = useStateRef(0);
    useEffect(() => { loadInitialData(setHotkeys, setPlugins, setSelectedIndex, markRestored); }, []);
    return {
        hotkeys, setHotkeys, hotkeysRef,
        plugins,
        selectedIndex, setSelectedIndex, selectedIndexRef,
        editModal, setEditModal, editModalRef,
        modalFieldIndex, setModalFieldIndex, modalFieldIndexRef
    };
}

function loadInitialData(setHotkeys, setPlugins, setSelectedIndex, markRestored) {
    loadHotkeysViewData().then(loaded => {
        setHotkeys(loaded.hotkeys);
        setPlugins(loaded.plugins);
        setSelectedIndex(prev => {
            markRestored();
            return loaded.hotkeys.length === 0 ? -1
                : prev >= 0 && prev < loaded.hotkeys.length ? prev : 0;
        });
    }).catch(() => {});
}

function useModalActions(d) {
    const getActions = useCallback(
        (pluginId, editingId) => resolveAvailableActions(d.plugins, d.hotkeysRef.current, pluginId, editingId),
        [d.plugins]
    );
    const openEditModal = useCallback((hotkey = null, keepPlugin = null) => {
        d.setEditModal(createEditModalState(hotkey, keepPlugin, getActions));
        d.setModalFieldIndex(0);
    }, [getActions]);
    const saveHotkey = useCallback(() => executeSave(d, getActions), [getActions]);
    const handlePluginChange = useCallback((id) => d.setEditModal(prev => changeEditModalPlugin(prev, id, getActions)), [getActions]);
    const handleActionChange = useCallback((action) => d.setEditModal(prev => prev ? { ...prev, action } : prev), []);
    const startRecording = useCallback(() => d.setEditModal(prev => prev ? { ...prev, recording: true, key: '' } : prev), []);
    const closeModal = useCallback(() => d.setEditModal(null), []);
    return { openEditModal, saveHotkey, getActions, handlePluginChange, handleActionChange, startRecording, closeModal };
}

function executeSave(d, getActions) {
    const modal = d.editModalRef.current;
    if (!modal?.key || !modal?.pluginId || !modal?.action) return;
    const nextHotkeys = buildSavedHotkeys(d.hotkeysRef.current, modal);
    const savedId = modal.hotkey?.id || nextHotkeys[nextHotkeys.length - 1]?.id;
    d.setHotkeys(nextHotkeys);
    void persistHotkeys(nextHotkeys);
    if (modal.hotkey) { d.setEditModal(null); return; }
    d.setSelectedIndex(nextHotkeys.length - 1);
    d.setEditModal(prev => nextEditModalState(prev, savedId, getActions));
    d.setModalFieldIndex(1);
}

function useListActions(d) {
    return {
        deleteSelected: useCallback(() => {
            const idx = d.selectedIndexRef.current;
            const hks = d.hotkeysRef.current;
            if (idx < 0 || idx >= hks.length) return;
            const next = removeHotkeyAtIndex(hks, idx);
            d.setHotkeys(next);
            d.setSelectedIndex(nextSelectedIndex(next, idx));
            void persistHotkeys(next);
        }, [])
    };
}

function useKeyboard(d, m, deleteSelected) {
    const handleKey = useCallback((e) => {
        const modal = d.editModalRef.current;
        if (modal) {
            if (modal.recording) {
                e.preventDefault();
                e.stopPropagation();
                const result = applyRecordingKey(modal, e);
                d.setEditModal(result.modal);
                if (result.advance) d.setModalFieldIndex(prev => prev + 1);
                return;
            }
            const action = resolveModalKeyAction(e, document.activeElement);
            if (action) applyModalAction(action, e, d.setEditModal, m.saveHotkey, d.modalFieldIndexRef, d.setModalFieldIndex);
            return;
        }
        dispatchListKey(e, d, m.openEditModal);
    }, [m.saveHotkey, m.openEditModal]);
    return { handleKey, isBlocking: useCallback(() => d.editModalRef.current !== null, []) };
}

function dispatchListKey(e, d, openEditModal) {
    const hks = d.hotkeysRef.current;
    const idx = d.selectedIndexRef.current;
    dispatchKey(e, withShiftVariants({
        ArrowUp: () => d.setSelectedIndex(i => Math.max(0, i - 1)),
        ArrowDown: () => d.setSelectedIndex(i => Math.min(hks.length - 1, i + 1)),
        Enter: () => { if (hks.length > 0 && idx >= 0) openEditModal(hks[idx]); },
    }));
}
