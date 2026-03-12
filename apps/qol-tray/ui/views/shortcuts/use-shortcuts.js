import { useState, useEffect, useCallback, useMemo } from 'preact/hooks';
import { useStateRef } from '../../hooks/useStateRef.js';
import { usePersistedId } from '../../hooks/usePersistedIndex.js';
import { withShiftVariants, dispatchKey } from '../../utils/keys.js';
import { matchesQuery } from '../../utils/collections.js';
import {
    loadShortcuts, createShortcut, updateShortcut,
    deleteShortcut, runShortcut, emptyShortcut
} from './data.js';

export function useShortcuts(searchQuery) {
    const d = useShortcutsData(searchQuery);
    const m = useModalActions(d);
    const { deleteById, runById } = useListActions(d);
    const { handleKey, isBlocking } = useKeyboard(d, m, deleteById, runById);
    return {
        filtered: d.filtered,
        selectedIndex: d.selectedIndex,
        setSelectedId: d.setSelectedId,
        editModal: d.editModal,
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
    const openEditModal = useCallback((shortcut = null) => {
        d.setEditModal({
            editing: !!shortcut,
            shortcut: shortcut ? { ...shortcut } : emptyShortcut()
        });
    }, []);

    const handleChange = useCallback((shortcut) => {
        d.setEditModal(prev => prev ? { ...prev, shortcut } : prev);
    }, []);

    const closeModal = useCallback(() => d.setEditModal(null), []);

    const save = useCallback(async () => {
        const modal = d.editModalRef.current;
        if (!modal) return;
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
    }, []);

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

    const handleKey = useCallback((e) => {
        if (d.editModalRef.current) {
            if (e.key === 'Escape') { e.preventDefault(); m.closeModal(); return; }
            if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') { e.preventDefault(); m.save(); return; }
            return;
        }
        const selected = filtered[selectedIndex];
        dispatchKey(e, withShiftVariants({
            ArrowUp: () => { const prev = filtered[selectedIndex - 1]; if (prev) setSelectedId(prev.id); },
            ArrowDown: () => { const next = filtered[selectedIndex + 1]; if (next) setSelectedId(next.id); },
            Enter: () => { if (selected) m.openEditModal(selected); },
            a: () => m.openEditModal(),
            r: () => { if (selected) runById(selected.id); },
            Delete: () => { if (selected) deleteById(selected.id); },
            Backspace: () => { if (selected) deleteById(selected.id); },
        }));
    }, [filtered, selectedIndex, setSelectedId, m.save, m.openEditModal, deleteById, runById]);

    return { handleKey, isBlocking: useCallback(() => d.editModalRef.current !== null, []) };
}
