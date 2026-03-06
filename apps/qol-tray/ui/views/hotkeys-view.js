import { html } from '../lib/html.js';
import { useEffect, useCallback } from 'preact/hooks';
import { useStateRef } from '../hooks/useStateRef.js';
import { usePersistedIndex } from '../hooks/usePersistedIndex.js';
import { useScrollIntoView } from '../hooks/useScrollIntoView.js';
import { useFooterShortcuts } from '../hooks/useFooterShortcuts.js';
import { PageHeader } from '../components/PageHeader.js';
import { withShiftVariants, dispatchKey } from '../utils/keys.js';
import {
    buildSavedHotkeys,
    getAvailableActions as resolveAvailableActions,
    loadHotkeysViewData,
    nextSelectedIndex,
    persistHotkeys,
    removeHotkeyAtIndex
} from './hotkeys/data.js';
import {
    applyRecordingKey,
    changeEditModalPlugin,
    createEditModalState,
    HotkeyEditModal,
    nextEditModalState
} from './hotkeys/modal.js';

const SHORTCUTS = [
    { key: '↑↓', label: 'navigate' },
    { key: 'Enter', label: 'edit' },
    { key: 'a', label: 'add' },
    { key: 'd', label: 'delete' }
];

function getActionLabel(plugin, actionId) {
    if (!plugin) return actionId;
    const action = plugin.actions?.find(a => a.id === actionId);
    return action ? action.label : actionId;
}

export function HotkeysView() {
    const [hotkeys, setHotkeys, hotkeysRef] = useStateRef([]);
    const [plugins, setPlugins] = useStateRef([]);
    const [selectedIndex, setSelectedIndex, selectedIndexRef, hotkeysMarkRestored] = usePersistedIndex('hotkeys-selected-index', -1);
    const [editModal, setEditModal, editModalRef] = useStateRef(null); // null | { hotkey, pluginId, action, key, recording }
    const [modalFieldIndex, setModalFieldIndex, modalFieldIndexRef] = useStateRef(0);

    useFooterShortcuts(SHORTCUTS);

    useEffect(() => {
        (async () => {
            try {
                const loaded = await loadHotkeysViewData();
                const hks = loaded.hotkeys;
                setHotkeys(hks);
                setPlugins(loaded.plugins);
                setSelectedIndex(prev => {
                    hotkeysMarkRestored();
                    if (hks.length === 0) return -1;
                    return prev >= 0 && prev < hks.length ? prev : 0;
                });
            } catch {}
        })();
    }, []);

    useScrollIntoView('.hotkey-row.selected', [selectedIndex]);

    const getAvailableActions = useCallback((pluginId, editingId) => {
        return resolveAvailableActions(plugins, hotkeysRef.current, pluginId, editingId);
    }, [plugins]);

    const openEditModal = useCallback((hotkey = null, keepPlugin = null) => {
        setEditModal(createEditModalState(hotkey, keepPlugin, getAvailableActions));
        setModalFieldIndex(0);
    }, [getAvailableActions]);

    const handlePluginChange = useCallback((pluginId) => {
        setEditModal(prev => changeEditModalPlugin(prev, pluginId, getAvailableActions));
    }, [getAvailableActions]);

    const saveHotkey = useCallback(() => {
        const modal = editModalRef.current;
        if (!modal?.key || !modal?.pluginId || !modal?.action) return;
        const nextHotkeys = buildSavedHotkeys(hotkeysRef.current, modal);
        const savedId = modal.hotkey?.id || nextHotkeys[nextHotkeys.length - 1]?.id;

        setHotkeys(nextHotkeys);
        void persistHotkeys(nextHotkeys);

        if (modal.hotkey) {
            setEditModal(null);
            return;
        }

        setSelectedIndex(nextHotkeys.length - 1);
        setEditModal(prev => nextEditModalState(prev, savedId, getAvailableActions));
        setModalFieldIndex(1);
    }, [getAvailableActions]);

    const deleteSelected = useCallback(() => {
        const idx = selectedIndexRef.current;
        const hks = hotkeysRef.current;
        if (idx < 0 || idx >= hks.length) return;
        const next = removeHotkeyAtIndex(hks, idx);
        setHotkeys(next);
        setSelectedIndex(nextSelectedIndex(next, idx));
        void persistHotkeys(next);
    }, []);

    const handleRecordingKey = useCallback((e) => {
        e.preventDefault();
        e.stopPropagation();
        const result = applyRecordingKey(editModalRef.current, e);
        setEditModal(result.modal);
        if (result.advance) setModalFieldIndex(prev => prev + 1);
    }, []);

    const handleKey = useCallback((e) => {
        const modal = editModalRef.current;
        if (modal) {
            if (modal.recording) { handleRecordingKey(e); return; }
            if (e.key === 'Escape') { e.preventDefault(); setEditModal(null); return; }
            if (e.key === 'Enter' && e.ctrlKey) { e.preventDefault(); saveHotkey(); return; }
            if (e.key === 'Enter') {
                e.preventDefault();
                const active = document.activeElement;
                if (active?.id === 'hotkey-key') { setEditModal(prev => prev ? { ...prev, recording: true, key: '' } : prev); return; }
                if (active?.classList.contains('modal-cancel')) { setEditModal(null); return; }
                if (active?.classList.contains('modal-save')) { saveHotkey(); return; }
                const fields = Array.from(document.querySelectorAll('.edit-modal [tabindex]'));
                const idx = fields.indexOf(active);
                if (idx >= 0 && idx + 1 < fields.length) fields[idx + 1].focus();
                return;
            }
            if (e.key === 'Tab') {
                e.preventDefault();
                const fields = Array.from(document.querySelectorAll('.edit-modal [tabindex]'));
                if (fields.length === 0) return;
                const dir = e.shiftKey ? -1 : 1;
                const next = (modalFieldIndexRef.current + dir + fields.length) % fields.length;
                setModalFieldIndex(next);
                fields[next]?.focus();
            }
            return;
        }
        const hks = hotkeysRef.current;
        const idx = selectedIndexRef.current;
        dispatchKey(e, withShiftVariants({
            ArrowUp: () => setSelectedIndex(i => Math.max(0, i - 1)),
            ArrowDown: () => setSelectedIndex(i => Math.min(hks.length - 1, i + 1)),
            Enter: () => { if (hks.length > 0 && idx >= 0) openEditModal(hks[idx]); },
            a: () => openEditModal(),
            d: deleteSelected,
        }));
    }, [handleRecordingKey, saveHotkey, openEditModal, deleteSelected]);

    const isBlocking = useCallback(() => editModalRef.current !== null, []);

    HotkeysView.handleKey = handleKey;
    HotkeysView.isBlocking = isBlocking;

    return html`
        <div class="view-container">
            <${PageHeader}
                title="Hotkeys"
                subtitle="Configure global keyboard shortcuts for plugin actions"
            />
            <div class="view-body">
                <div class="hotkeys-list table-list">
                    ${hotkeys.length === 0 && html`
                        <div class="empty">No hotkeys configured. Press <kbd>a</kbd> to add one.</div>
                    `}
                    ${hotkeys.length > 0 && html`
                        <div class="hotkey-header table-list-header table-grid">
                            <span class="col-key table-cell">Shortcut</span>
                            <span class="col-plugin table-cell">Plugin</span>
                            <span class="col-action table-cell">Action</span>
                        </div>
                    `}
                    ${hotkeys.map((hk, index) => {
                        const plugin = plugins.find(p => p.id === hk.plugin_id);
                        const status = plugin?.status || 'installed';
                        const isSelected = index === selectedIndex;
                        return html`
                            <div key=${hk.id}
                                 class="hotkey-row table-list-row table-grid"
                                 data-status="${status}"
                                 data-selected="${isSelected ? 'true' : 'false'}"
                                 data-index="${index}"
                                 onClick=${() => {
                                     if (index !== selectedIndex) setSelectedIndex(index);
                                     else openEditModal(hk);
                                 }}>
                                <span class="col-key table-cell"><kbd>${hk.key}</kbd></span>
                                <span class="col-plugin table-cell">${plugin?.name || hk.plugin_id}</span>
                                <span class="col-action table-cell">${getActionLabel(plugin, hk.action)}</span>
                            </div>
                        `;
                    })}
                </div>
            </div>
            ${editModal && html`
                <${HotkeyEditModal}
                    modal=${editModal}
                    plugins=${plugins}
                    onPluginChange=${handlePluginChange}
                    onActionChange=${(action) => setEditModal(prev => prev ? { ...prev, action } : prev)}
                    onStartRecording=${() => setEditModal(prev => prev ? { ...prev, recording: true, key: '' } : prev)}
                    onClose=${() => setEditModal(null)}
                    onSave=${saveHotkey}
                />
            `}
        </div>
    `;
}
