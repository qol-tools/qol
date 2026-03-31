import { useState, useEffect, useCallback, useMemo } from 'preact/hooks';
import { useStateRef } from '../../hooks/useStateRef.js';
import { usePersistedId } from '../../hooks/usePersistedIndex.js';
import { useListKeyboard } from '../../hooks/useListKeyboard.js';
import { useModalKeyboard } from '../../hooks/useModalKeyboard.js';
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

    const listHandler = useListKeyboard({
        surfaceSelector: '.shortcuts-list [data-selected-surface]',
        itemCount: filtered.length,
        selectedIndex,
        setSelectedIndex,
        onAdd: m.openEditModal,
        onDelete: useCallback(() => { if (selected) deleteById(selected.id); }, [selected, deleteById]),
        onEdit: useCallback(() => { if (selected) m.openEditModal(selected); }, [selected, m.openEditModal]),
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
