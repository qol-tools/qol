import { useState, useEffect, useCallback, useMemo, useRef } from 'preact/hooks';
import { useStateRef } from '../../lib/hooks/useStateRef.js';
import { usePersistedId } from '../../lib/hooks/usePersistedIndex.js';
import { useListKeyboard } from '../../lib/hooks/useListKeyboard.js';
import { useModalKeyboard } from '../../lib/hooks/useModalKeyboard.js';
import { diveViaSelector } from '../../lib/world-navigation-singleton.js';
import { isShortcutValid } from './validation.js';

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
            ? shortcuts.filter(s => matchesQuery([s.name, s.id, s.action.url], searchQuery))
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

    const openEditModal = useCallback((shortcut = null) => {
        cancelPending();
        d.setEditModal({
            editing: !!shortcut,
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
            const config = modal.editing
                ? await updateShortcut(modal.shortcut)
                : await createShortcut(modal.shortcut);
            d.setShortcuts(config.shortcuts || []);
            if (!modal.editing) d.setSelectedId(modal.shortcut.id);
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
    const { filtered, selectedIndex, setSelectedId } = d;

    const setSelectedIndex = useCallback((idxOrFn) => {
        const idx = typeof idxOrFn === 'function' ? idxOrFn(d.selectedIndex) : idxOrFn;
        const item = filtered[idx];
        if (item) setSelectedId(item.id);
    }, [filtered, d.selectedIndex, setSelectedId]);

    const selected = filtered[selectedIndex];

    const modalNav = useModalKeyboard({
        onSave: m.save,
        onClose: m.closeModal,
    });

    const onAdd = useCallback(() => {
        m.openEditModal();
        diveViaSelector(SHORTCUTS_EDITOR_DIVE_SELECTOR);
    }, [m.openEditModal]);

    const listHandler = useListKeyboard({
        itemCount: filtered.length,
        selectedIndex,
        onAdd,
        onDelete: useCallback(() => { if (selected) deleteById(selected.id); }, [selected, deleteById]),
    });

    const handleKey = useCallback((e) => {
        if (d.editModalRef.current) {
            modalNav.handleKey(e);
            return;
        }
        if (e.key === 'r' || e.key === 'R') {
            if (selected) { e.preventDefault(); runById(selected.id); }
            return;
        }
        listHandler(e);
    }, [listHandler, modalNav.handleKey, selected, runById]);

    return {
        handleKey,
        isBlocking: useCallback(() => d.editModalRef.current !== null, []),
        modalNav,
    };
}
