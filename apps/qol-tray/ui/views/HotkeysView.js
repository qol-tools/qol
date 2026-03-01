import { html } from '../lib/html.js';
import { useEffect, useCallback } from 'preact/hooks';
import { useStateRef } from '../hooks/useStateRef.js';
import { usePersistedIndex } from '../hooks/usePersistedIndex.js';
import { useScrollIntoView } from '../hooks/useScrollIntoView.js';
import { Modal } from '../components/ModalPreact.js';
import { useFooterShortcuts } from '../hooks/useFooterShortcuts.js';
import { apiJson, apiResponse, jsonRequest } from '../api/client.js';
import { parseInstalledPlugins } from '../utils/plugins.js';

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

function formatKeyEvent(e) {
    const parts = [];
    if (e.ctrlKey) parts.push('Ctrl');
    if (e.altKey) parts.push('Alt');
    if (e.shiftKey) parts.push('Shift');
    if (e.metaKey) parts.push('Super');
    if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) return parts.join('+') || '';
    const key = getKeyName(e.code);
    if (key) parts.push(key);
    return parts.join('+');
}

function getKeyName(code) {
    if (code.startsWith('Key')) return code.slice(3);
    if (code.startsWith('Digit')) return code.slice(5);
    if (code.startsWith('Numpad')) return code;
    const map = {
        Space: 'Space', Enter: 'Enter', Escape: 'Escape', Tab: 'Tab',
        Backspace: 'Backspace', Delete: 'Delete', Insert: 'Insert',
        Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown',
        ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right',
        F1: 'F1', F2: 'F2', F3: 'F3', F4: 'F4', F5: 'F5', F6: 'F6',
        F7: 'F7', F8: 'F8', F9: 'F9', F10: 'F10', F11: 'F11', F12: 'F12',
        PrintScreen: 'PrintScreen', Pause: 'Pause'
    };
    return map[code] || null;
}

export function HotkeysView() {
    const [hotkeys, setHotkeys, hotkeysRef] = useStateRef([]);
    const [plugins, setPlugins] = useStateRef([]);
    const [selectedIndex, setSelectedIndex, selectedIndexRef, hotkeysMarkRestored] = usePersistedIndex('hotkeys-selected-index', -1);
    const [editModal, setEditModal, editModalRef] = useStateRef(null); // null | { hotkey, pluginId, action, key, recording }
    const [modalFieldIndex, setModalFieldIndex, modalFieldIndexRef] = useStateRef(0);

    useFooterShortcuts(SHORTCUTS);

    // Load data
    useEffect(() => {
        (async () => {
            try {
                const [hotkeysConfig, installedPayload] = await Promise.all([
                    apiJson('/api/hotkeys'),
                    apiJson('/api/installed')
                ]);
                const hks = hotkeysConfig.hotkeys || [];
                setHotkeys(hks);
                setPlugins(parseInstalledPlugins(installedPayload));
                setSelectedIndex(prev => {
                    hotkeysMarkRestored();
                    if (hks.length === 0) return -1;
                    return prev >= 0 && prev < hks.length ? prev : 0;
                });
            } catch {}
        })();
    }, []);

    useScrollIntoView('.hotkey-row.selected', [selectedIndex]);

    // Persist
    const persistHotkeys = useCallback(async (hks) => {
        try { await apiResponse('/api/hotkeys', jsonRequest('PUT', { hotkeys: hks })); } catch {}
    }, []);

    // Available actions for a plugin (excluding already-assigned ones)
    const getAvailableActions = useCallback((pluginId, editingId) => {
        const plugin = plugins.find(p => p.id === pluginId);
        if (!plugin?.actions?.length) return [{ id: 'run', label: 'Run' }];
        const assigned = hotkeysRef.current
            .filter(h => h.plugin_id === pluginId && h.id !== editingId)
            .map(h => h.action);
        return plugin.actions.filter(a => !assigned.includes(a.id));
    }, [plugins]);

    // Modal open
    const openEditModal = useCallback((hotkey = null, keepPlugin = null) => {
        const pluginId = keepPlugin || hotkey?.plugin_id || '';
        const available = pluginId ? getAvailableActions(pluginId, hotkey?.id) : [];
        setEditModal({
            hotkey,
            pluginId,
            action: hotkey?.action || available[0]?.id || '',
            key: hotkey?.key || '',
            recording: false,
            availableActions: available
        });
        setModalFieldIndex(0);
    }, [getAvailableActions]);

    // Modal plugin change
    const handlePluginChange = useCallback((pluginId) => {
        setEditModal(prev => {
            if (!prev) return prev;
            const available = getAvailableActions(pluginId, prev.hotkey?.id);
            return { ...prev, pluginId, action: available[0]?.id || '', availableActions: available };
        });
    }, [getAvailableActions]);

    // Save hotkey — stable via ref
    const saveHotkey = useCallback(() => {
        const modal = editModalRef.current;
        if (!modal?.key || !modal?.pluginId || !modal?.action) return;
        const entry = {
            id: modal.hotkey?.id || `hk-${Date.now()}`,
            key: modal.key,
            plugin_id: modal.pluginId,
            action: modal.action,
            enabled: true
        };
        const isEditing = !!modal.hotkey;
        setHotkeys(prev => {
            const next = isEditing
                ? prev.map(h => h.id === modal.hotkey.id ? entry : h)
                : [...prev, entry];
            if (!isEditing) setSelectedIndex(next.length - 1);
            persistHotkeys(next);
            return next;
        });
        if (isEditing) { setEditModal(null); return; }
        const available = getAvailableActions(modal.pluginId, entry.id);
        if (available.length === 0) { setEditModal(null); return; }
        setEditModal(prev => ({
            ...prev, hotkey: null, key: '', action: available[0]?.id || '',
            recording: false, availableActions: available
        }));
        setModalFieldIndex(1);
    }, [persistHotkeys, getAvailableActions]);

    // Delete — stable via refs
    const deleteSelected = useCallback(() => {
        const idx = selectedIndexRef.current;
        const hks = hotkeysRef.current;
        if (idx < 0 || idx >= hks.length) return;
        const next = hks.filter((_, i) => i !== idx);
        setHotkeys(next);
        setSelectedIndex(Math.min(idx, Math.max(0, next.length - 1)));
        persistHotkeys(next);
    }, [persistHotkeys]);

    // Key recording handler
    const handleRecordingKey = useCallback((e) => {
        e.preventDefault();
        e.stopPropagation();
        if (e.key === 'Escape') {
            setEditModal(prev => prev ? { ...prev, recording: false } : prev);
            return;
        }
        const MODIFIERS = ['Control', 'Alt', 'Shift', 'Meta'];
        if (MODIFIERS.includes(e.key)) {
            const current = formatKeyEvent(e);
            if (current) setEditModal(prev => prev ? { ...prev, key: current } : prev);
            return;
        }
        const key = formatKeyEvent(e);
        const MOD_NAMES = ['Ctrl', 'Alt', 'Shift', 'Super'];
        if (key && !MOD_NAMES.includes(key)) {
            setEditModal(prev => prev ? { ...prev, key, recording: false } : prev);
            setModalFieldIndex(prev => prev + 1);
        }
    }, []);

    // Keyboard handler — stable via refs
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
        const handlers = {
            ArrowUp: () => setSelectedIndex(i => Math.max(0, i - 1)),
            ArrowDown: () => setSelectedIndex(i => Math.min(hks.length - 1, i + 1)),
            Enter: () => { if (hks.length > 0 && idx >= 0) openEditModal(hks[idx]); },
            a: () => openEditModal(),
            A: () => openEditModal(),
            d: deleteSelected,
            D: deleteSelected,
        };
        const handler = handlers[e.key];
        if (handler) { e.preventDefault(); handler(); }
    }, [handleRecordingKey, saveHotkey, openEditModal, deleteSelected]);

    const isBlocking = useCallback(() => editModalRef.current !== null, []);

    HotkeysView.handleKey = handleKey;
    HotkeysView.isBlocking = isBlocking;

    return html`
        <div class="view-container">
            <header>
                <h1>Hotkeys</h1>
                <p>Configure global keyboard shortcuts for plugin actions</p>
            </header>
            <div class="view-body">
                <div class="hotkeys-list">
                    ${hotkeys.length === 0 && html`
                        <div class="empty">No hotkeys configured. Press <kbd>a</kbd> to add one.</div>
                    `}
                    ${hotkeys.length > 0 && html`
                        <div class="hotkey-header">
                            <span class="col-key">Shortcut</span>
                            <span class="col-plugin">Plugin</span>
                            <span class="col-action">Action</span>
                        </div>
                    `}
                    ${hotkeys.map((hk, index) => {
                        const plugin = plugins.find(p => p.id === hk.plugin_id);
                        return html`
                            <div key=${hk.id} class="hotkey-row ${index === selectedIndex ? 'selected' : ''}"
                                 data-index="${index}"
                                 onClick=${() => {
                                     if (index !== selectedIndex) setSelectedIndex(index);
                                     else openEditModal(hk);
                                 }}>
                                <span class="col-key"><kbd>${hk.key}</kbd></span>
                                <span class="col-plugin">${plugin?.name || hk.plugin_id}</span>
                                <span class="col-action">${getActionLabel(plugin, hk.action)}</span>
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

function HotkeyEditModal({ modal, plugins, onPluginChange, onActionChange, onStartRecording, onClose, onSave }) {
    const isNew = !modal.hotkey;
    const title = isNew ? 'Add Hotkey' : 'Edit Hotkey';

    // Auto-focus plugin select on mount
    useEffect(() => {
        setTimeout(() => document.getElementById('hotkey-plugin')?.focus(), 0);
    }, []);

    return html`
        <${Modal} open=${true} onClose=${onClose} className="edit-modal">
            <div class="edit-modal-content">
                <h3>${title}</h3>
                <div class="form-group">
                    <label>Plugin</label>
                    <select id="hotkey-plugin" tabindex="1"
                            value=${modal.pluginId}
                            onChange=${(e) => onPluginChange(e.target.value)}>
                        <option value="">Select plugin...</option>
                        ${plugins.map(p => html`<option key=${p.id} value=${p.id}>${p.name}</option>`)}
                    </select>
                </div>
                <div class="form-group">
                    <label>Action</label>
                    <select id="hotkey-action" tabindex="2"
                            value=${modal.action}
                            onChange=${(e) => onActionChange(e.target.value)}>
                        ${modal.availableActions.length === 0
                            ? html`<option value="">All actions assigned</option>`
                            : modal.availableActions.map(a => html`<option key=${a.id} value=${a.id}>${a.label}</option>`)}
                    </select>
                </div>
                <div class="form-group">
                    <label>Shortcut <span class="hint">(Enter to record)</span></label>
                    <div class="key-input-row">
                        <input type="text" id="hotkey-key" tabindex="3"
                               value=${modal.key} readonly
                               class=${modal.recording ? 'recording' : ''}
                               placeholder=${modal.recording ? 'Press keys... (Esc to cancel)' : 'Press Enter to record'}
                               onClick=${onStartRecording} />
                    </div>
                </div>
                <div class="modal-buttons">
                    <button class="btn btn-ghost modal-cancel" tabindex="4" onClick=${onClose}>Cancel <kbd>Esc</kbd></button>
                    <button class="btn btn-primary modal-save" tabindex="5" onClick=${onSave}>Save <kbd>Ctrl+Enter</kbd></button>
                </div>
            </div>
        <//>
    `;
}
