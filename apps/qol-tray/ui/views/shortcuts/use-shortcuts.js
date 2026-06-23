import { useState, useEffect, useCallback, useMemo, useRef } from 'preact/hooks';
import { useStateRef } from '../../lib/hooks/useStateRef.js';
import { usePersistedId } from '../../lib/hooks/usePersistedIndex.js';
import { useListEditorKeyboard } from '../../lib/hooks/useListEditorKeyboard.js';
import { diveViaSelector } from '../../lib/world-navigation-singleton.js';
import { isShortcutValid } from './validation.js';
import { deriveShortcutId } from './derive-id.js';
import { actionSearchText, isManagedPluginShortcut } from './action.js';

const AUTO_SAVE_DEBOUNCE_MS = 400;

const SHORTCUTS_EDITOR_DIVE_SELECTOR = '[data-view-id="shortcuts"]';
import { matchesQuery } from '../../utils/collections.js';
import {
    loadShortcuts, createShortcut, updateShortcut,
    deleteShortcut, runShortcut, emptyShortcut
} from './data.js';

export function useShortcuts(searchQuery) {
    const d = useShortcutsData(searchQuery);
    const m = useModalActions(d);
    const { deleteById, runById } = useListActions(d);
    const { handleKey, isBlocking, modalNav } = useKeyboard(d, m, deleteById, runById);
    return {
        filtered: d.filtered,
        selectedIndex: d.selectedIndex,
        setSelectedId: d.setSelectedId,
        editModal: d.editModal,
        fieldProps: modalNav.fieldProps,
        openEditModal: m.openEditModal,
        handleModalChange: m.handleChange,
        closeModal: m.closeModal,
        saveShortcut: m.save,
        deleteById,
        runById,
        handleKey,
        isBlocking
    };
}

function useShortcutsData(searchQuery) {
    const [shortcuts, setShortcuts] = useState([]);
    const [selectedId, setSelectedId, selectedIdRef, markRestored] = usePersistedId('shortcuts-selected-id');
    const [editModal, setEditModal, editModalRef] = useStateRef(null);

    const filtered = useMemo(
        () => searchQuery
            ? shortcuts.filter(s => matchesQuery([s.name, s.id, actionSearchText(s.action)], searchQuery))
            : shortcuts,
        [shortcuts, searchQuery]
    );

    const selectedIndex = useMemo(() => {
        if (!selectedId) return filtered.length > 0 ? 0 : -1;
        const idx = filtered.findIndex(s => s.id === selectedId);
        if (idx >= 0) return idx;
        return filtered.length > 0 ? 0 : -1;
    }, [filtered, selectedId]);

    useEffect(() => {
        loadShortcuts().then(config => {
            const list = config.shortcuts || [];
            setShortcuts(list);
            markRestored();
            if (!selectedIdRef.current || !list.some(s => s.id === selectedIdRef.current)) {
                setSelectedId(list[0]?.id ?? null);
            }
        }).catch(e => console.error('Failed to load shortcuts:', e));
    }, []);

    return {
        shortcuts, setShortcuts,
        filtered, selectedIndex,
        selectedId, setSelectedId,
        editModal, setEditModal, editModalRef,
    };
}

function hostFromUrl(url) {
    try { return new URL(url).host; } catch { return ''; }
}

function useModalActions(d) {
    const pendingTimer = useRef(null);

    const cancelPending = useCallback(() => {
        if (pendingTimer.current !== null) {
            clearTimeout(pendingTimer.current);
            pendingTimer.current = null;
        }
    }, []);

    const autoSave = useCallback(async (shortcut) => {
        if (!isShortcutValid(shortcut)) return;
        try {
            const config = await updateShortcut(shortcut);
            d.setShortcuts(config.shortcuts || []);
        } catch {
            // Surfaced by the global fetch toast; intermediate edits should not
            // re-alert during typing.
        }
    }, []);

    useEffect(() => cancelPending, [cancelPending]);

    const openEditModal = useCallback((shortcut = null, opts = {}) => {
        cancelPending();
        if (isManagedPluginShortcut(shortcut)) return;
        d.setEditModal({
            editing: opts.editing ?? !!shortcut,
            shortcut: shortcut ? { ...shortcut } : emptyShortcut()
        });
    }, [cancelPending]);

    const handleChange = useCallback((shortcut) => {
        d.setEditModal(prev => prev ? { ...prev, shortcut } : prev);
        if (!d.editModalRef.current?.editing) return;
        cancelPending();
        pendingTimer.current = setTimeout(() => {
            pendingTimer.current = null;
            autoSave(shortcut);
        }, AUTO_SAVE_DEBOUNCE_MS);
    }, [autoSave, cancelPending]);

    const closeModal = useCallback(() => {
        const modal = d.editModalRef.current;
        const hadPending = pendingTimer.current !== null;
        cancelPending();
        if (modal?.editing && hadPending) autoSave(modal.shortcut);
        d.setEditModal(null);
    }, [autoSave, cancelPending]);

    const save = useCallback(async () => {
        const modal = d.editModalRef.current;
        if (!modal) return;
        cancelPending();
        try {
            let shortcut = modal.shortcut;
            if (!modal.editing) {
                const host = shortcut.action?.type === 'open_url'
                    ? hostFromUrl(shortcut.action.url) : '';
                const existing = d.shortcuts.map(s => s.id);
                shortcut = { ...shortcut, id: deriveShortcutId(shortcut.name, existing, host) };
            }
            const config = modal.editing
                ? await updateShortcut(modal.shortcut)
                : await createShortcut(shortcut);
            d.setShortcuts(config.shortcuts || []);
            if (!modal.editing) d.setSelectedId(shortcut.id);
            d.setEditModal(null);
        } catch (e) {
            alert(e.message || 'Failed to save shortcut');
        }
    }, [cancelPending]);

    return { openEditModal, handleChange, closeModal, save };
}

function useListActions(d) {
    const deleteById = useCallback(async (id) => {
        if (!id) return;
        const shortcut = d.shortcuts.find(s => s.id === id);
        if (isManagedPluginShortcut(shortcut)) return;
        try {
            const config = await deleteShortcut(id);
            d.setShortcuts(config.shortcuts || []);
            d.setSelectedId(prev => prev === id ? null : prev);
        } catch (e) { console.error('Failed to delete shortcut:', e); }
    }, []);

    const runById = useCallback(async (id) => {
        if (!id) return;
        try { await runShortcut(id); } catch (e) { alert(e.message || 'Failed to run shortcut'); }
    }, []);

    return { deleteById, runById };
}

function useKeyboard(d, m, deleteById, runById) {
    const { filtered, selectedIndex } = d;
    const selected = filtered[selectedIndex];

    const onAdd = useCallback(() => {
        m.openEditModal();
        diveViaSelector(SHORTCUTS_EDITOR_DIVE_SELECTOR);
    }, [m.openEditModal]);

    const onDelete = useCallback(() => {
        if (selected) deleteById(selected.id);
    }, [selected, deleteById]);

    const listIntercept = useCallback((e) => {
        if ((e.key === 'r' || e.key === 'R') && !e.ctrlKey && !e.metaKey && !e.altKey) {
            if (selected) { e.preventDefault(); runById(selected.id); }
            return true;
        }
        return false;
    }, [selected, runById]);

    return useListEditorKeyboard({
        editModalRef: d.editModalRef,
        onModalSave: m.save,
        onModalClose: m.closeModal,
        itemCount: filtered.length,
        selectedIndex,
        onAdd,
        onDelete,
        listIntercept,
    });
}
