import { useEffect, useCallback } from 'preact/hooks';
import { useStateRef } from '../../lib/hooks/useStateRef.js';
import { usePersistedIndex } from '../../lib/hooks/usePersistedIndex.js';
import { useSSEDebounce } from '../../hooks/useSSEDebounce.js';
import { useListKeyboard } from '../../lib/hooks/useListKeyboard.js';
import { useModalKeyboard } from '../../lib/hooks/useModalKeyboard.js';
import {
    buildSavedHotkeys,
    getAvailableActions as resolveAvailableActions,
    loadHotkeysViewData,
    loadPlugins,
    loadRegistrationErrors,
    nextSelectedIndex,
    persistHotkeys,
    removeHotkeyAtIndex
} from './data.js';
import {
    applyRecordingKey,
    changeEditModalPlugin,
    createEditModalState,
} from './modal.js';

export function useHotkeys() {
    const d = useHotkeysData();
    const m = useModalActions(d);
    const { deleteSelected } = useListActions(d);
    const { handleKey, isBlocking, modalNav } = useKeyboard(d, m, deleteSelected);
    return {
        hotkeys: d.hotkeys,
        plugins: d.plugins,
        registrationErrors: d.registrationErrors,
        selectedIndex: d.selectedIndex,
        editModal: d.editModal,
        fieldProps: modalNav.fieldProps,
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
    const [registrationErrors, setRegistrationErrors] = useStateRef([]);
    const [selectedIndex, setSelectedIndex, selectedIndexRef, markRestored] = usePersistedIndex('hotkeys-selected-index', -1);
    const [editModal, setEditModal, editModalRef] = useStateRef(null);
    useEffect(() => { loadInitialData(setHotkeys, setPlugins, setSelectedIndex, markRestored); }, []);
    const refreshPlugins = useCallback(() => { loadPlugins().then(setPlugins).catch(() => {}); }, []);
    const refreshErrors = useCallback(() => { loadRegistrationErrors().then(setRegistrationErrors).catch(() => {}); }, []);
    useSSEDebounce('plugins_changed', refreshPlugins);
    useEffect(() => { refreshErrors(); }, []);
    return {
        hotkeys, setHotkeys, hotkeysRef,
        plugins, registrationErrors, refreshErrors,
        selectedIndex, setSelectedIndex, selectedIndexRef,
        editModal, setEditModal, editModalRef,
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
    d.setHotkeys(nextHotkeys);
    d.hotkeysRef.current = nextHotkeys;
    persistHotkeys(nextHotkeys).then(() => setTimeout(d.refreshErrors, 200));
    if (modal.hotkey) {
        d.setEditModal(null);
        return;
    }
    d.setSelectedIndex(nextHotkeys.length - 1);
    const remaining = getActions(modal.pluginId);
    d.setEditModal({
        ...modal,
        hotkey: null,
        key: '',
        action: remaining[0]?.id || '',
        recording: false,
        availableActions: remaining,
    });
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
            persistHotkeys(next).then(() => setTimeout(d.refreshErrors, 200));
        }, [])
    };
}

function useKeyboard(d, m, deleteSelected) {
    const modalNav = useModalKeyboard({
        onSave: m.saveHotkey,
        onClose: m.closeModal,
    });

    const listHandler = useListKeyboard({
        itemCount: d.hotkeys.length,
        selectedIndex: d.selectedIndex,
        onAdd: m.openEditModal,
        onDelete: deleteSelected,
        onEdit: useCallback(() => {
            const hk = d.hotkeysRef.current[d.selectedIndexRef.current];
            if (hk) m.openEditModal(hk);
        }, [m.openEditModal]),
    });

    const handleKey = useCallback((e) => {
        const modal = d.editModalRef.current;
        if (modal) {
            if (modal.recording) {
                e.preventDefault();
                e.stopPropagation();
                const result = applyRecordingKey(modal, e);
                d.setEditModal(result.modal);
                return;
            }
            modalNav.handleKey(e);
            return;
        }
        listHandler(e);
    }, [listHandler, modalNav.handleKey]);

    return {
        handleKey,
        isBlocking: useCallback(() => d.editModalRef.current !== null, []),
        modalNav,
    };
}
