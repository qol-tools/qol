import { useEffect, useCallback } from 'preact/hooks';
import { useStateRef } from '../../lib/hooks/useStateRef.js';
import { usePersistedIndex } from '../../lib/hooks/usePersistedIndex.js';
import { useSSEDebounce } from '../../hooks/useSSEDebounce.js';
import { useListEditorKeyboard } from '../../lib/hooks/useListEditorKeyboard.js';
import { diveViaSelector } from '../../lib/world-navigation-singleton.js';

const HOTKEYS_EDITOR_DIVE_SELECTOR = '[data-view-id="hotkeys"]';
import { useRecorder } from './useRecorder.js';
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
    changeEditModalPlugin,
    createEditModalState,
} from './modal.js';

export function useHotkeys({ onAfterSave, onAfterClose } = {}) {
    const d = useHotkeysData();
    const recorder = useRecorder({
        onCapture: useCallback((key) => {
            d.setEditModal(prev => prev ? { ...prev, key } : prev);
        }, []),
    });
    const m = useModalActions(d, recorder);
    const saveAndExit = useCallback(() => {
        if (m.saveHotkey() === false) return false;
        onAfterSave?.();
        return true;
    }, [m.saveHotkey, onAfterSave]);
    const closeAndExit = useCallback(() => {
        recorder.cancel();
        m.closeModal();
        onAfterClose?.();
    }, [m.closeModal, onAfterClose, recorder.cancel]);
    const { deleteSelected } = useListActions(d);
    const { handleKey, isBlocking, modalNav } = useKeyboard(
        d,
        { ...m, saveHotkey: saveAndExit, closeModal: closeAndExit },
        deleteSelected,
        recorder
    );
    return {
        hotkeys: d.hotkeys,
        plugins: d.plugins,
        registrationErrors: d.registrationErrors,
        selectedIndex: d.selectedIndex,
        editModal: d.editModal,
        recorder,
        fieldProps: modalNav.fieldProps,
        setSelectedIndex: d.setSelectedIndex,
        openEditModal: m.openEditModal,
        handlePluginChange: m.handlePluginChange,
        handleActionChange: m.handleActionChange,
        handleEnabledChange: m.handleEnabledChange,
        startRecording: m.startRecording,
        closeModal: closeAndExit,
        saveHotkey: saveAndExit,
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

function useModalActions(d, recorder) {
    const getActions = useCallback(
        (pluginUid, editingId) => resolveAvailableActions(d.plugins, d.hotkeysRef.current, pluginUid, editingId),
        [d.plugins]
    );
    const openEditModal = useCallback((hotkey = null, keepPlugin = null) => {
        recorder.cancel();
        d.setEditModal(createEditModalState(hotkey, keepPlugin, getActions));
    }, [getActions, recorder.cancel]);
    const saveHotkey = useCallback(() => executeSave(d, recorder), [recorder]);
    const handlePluginChange = useCallback((uid) => d.setEditModal(prev => changeEditModalPlugin(prev, uid, getActions)), [getActions]);
    const handleActionChange = useCallback((action) => d.setEditModal(prev => prev ? { ...prev, action } : prev), []);
    const handleEnabledChange = useCallback((enabled) => d.setEditModal(prev => prev ? { ...prev, enabled } : prev), []);
    const startRecording = useCallback(() => recorder.start(''), [recorder.start]);
    const closeModal = useCallback(() => d.setEditModal(null), []);
    return { openEditModal, saveHotkey, getActions, handlePluginChange, handleActionChange, handleEnabledChange, startRecording, closeModal };
}

function executeSave(d, recorder) {
    const modal = d.editModalRef.current;
    if (!modal?.key || !modal?.pluginUid || !modal?.action) return false;
    recorder.cancel();
    const nextHotkeys = buildSavedHotkeys(d.hotkeysRef.current, modal);
    d.setHotkeys(nextHotkeys);
    d.hotkeysRef.current = nextHotkeys;
    persistHotkeys(nextHotkeys).then(() => setTimeout(d.refreshErrors, 200));
    if (!modal.hotkey) d.setSelectedIndex(nextHotkeys.length - 1);
    d.setEditModal(null);
    return true;
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

function useKeyboard(d, m, deleteSelected, recorder) {
    const onAdd = useCallback(() => {
        m.openEditModal();
        diveViaSelector(HOTKEYS_EDITOR_DIVE_SELECTOR);
    }, [m.openEditModal]);

    return useListEditorKeyboard({
        editModalRef: d.editModalRef,
        onModalSave: m.saveHotkey,
        onModalClose: m.closeModal,
        itemCount: d.hotkeys.length,
        selectedIndex: d.selectedIndex,
        onAdd,
        onDelete: deleteSelected,
        preIntercept: recorder.handleKey,
    });
}
