import { html } from '../../lib/html.js';
import { useEffect } from 'preact/hooks';
import { Modal } from '../../components/ModalPreact.js';

const MODIFIER_KEYS = ['Control', 'Alt', 'Shift', 'Meta'];
const MODIFIER_NAMES = ['Ctrl', 'Alt', 'Shift', 'Super'];

export function HotkeyEditModal({
    modal,
    plugins,
    onPluginChange,
    onActionChange,
    onStartRecording,
    onClose,
    onSave
}) {
    const title = modal.hotkey ? 'Edit Hotkey' : 'Add Hotkey';

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

export function createEditModalState(hotkey, keepPlugin, getAvailableActions) {
    const pluginId = keepPlugin || hotkey?.plugin_id || '';
    const availableActions = pluginId ? getAvailableActions(pluginId, hotkey?.id) : [];
    return {
        hotkey,
        pluginId,
        action: hotkey?.action || availableActions[0]?.id || '',
        key: hotkey?.key || '',
        recording: false,
        availableActions
    };
}

export function changeEditModalPlugin(previous, pluginId, getAvailableActions) {
    if (!previous) {
        return previous;
    }

    const availableActions = getAvailableActions(pluginId, previous.hotkey?.id);
    return {
        ...previous,
        pluginId,
        action: availableActions[0]?.id || '',
        availableActions
    };
}

export function nextEditModalState(previous, entryId, getAvailableActions) {
    if (previous.hotkey) {
        return null;
    }

    const availableActions = getAvailableActions(previous.pluginId, entryId);
    if (availableActions.length === 0) {
        return null;
    }

    return {
        ...previous,
        hotkey: null,
        key: '',
        action: availableActions[0]?.id || '',
        recording: false,
        availableActions
    };
}

export function applyRecordingKey(modal, event) {
    if (event.key === 'Escape') {
        return { modal: stopRecording(modal), advance: false };
    }
    if (MODIFIER_KEYS.includes(event.key)) {
        return { modal: updateRecordedKey(modal, formatKeyEvent(event)), advance: false };
    }

    const key = formatKeyEvent(event);
    if (!key || MODIFIER_NAMES.includes(key)) {
        return { modal, advance: false };
    }

    return {
        modal: {
            ...modal,
            key,
            recording: false
        },
        advance: true
    };
}

function stopRecording(modal) {
    return {
        ...modal,
        recording: false
    };
}

function updateRecordedKey(modal, key) {
    if (!key) {
        return modal;
    }

    return {
        ...modal,
        key
    };
}

function formatKeyEvent(event) {
    const parts = [];
    if (event.ctrlKey) parts.push('Ctrl');
    if (event.altKey) parts.push('Alt');
    if (event.shiftKey) parts.push('Shift');
    if (event.metaKey) parts.push('Super');
    if (MODIFIER_KEYS.includes(event.key)) {
        return parts.join('+') || '';
    }

    const key = getKeyName(event.code);
    if (key) parts.push(key);
    return parts.join('+');
}

function getKeyName(code) {
    if (code.startsWith('Key')) return code.slice(3);
    if (code.startsWith('Digit')) return code.slice(5);
    if (code.startsWith('Numpad')) return code;

    const map = {
        Space: 'Space',
        Enter: 'Enter',
        Escape: 'Escape',
        Tab: 'Tab',
        Backspace: 'Backspace',
        Delete: 'Delete',
        Insert: 'Insert',
        Home: 'Home',
        End: 'End',
        PageUp: 'PageUp',
        PageDown: 'PageDown',
        ArrowUp: 'Up',
        ArrowDown: 'Down',
        ArrowLeft: 'Left',
        ArrowRight: 'Right',
        F1: 'F1',
        F2: 'F2',
        F3: 'F3',
        F4: 'F4',
        F5: 'F5',
        F6: 'F6',
        F7: 'F7',
        F8: 'F8',
        F9: 'F9',
        F10: 'F10',
        F11: 'F11',
        F12: 'F12',
        PrintScreen: 'PrintScreen',
        Pause: 'Pause'
    };

    return map[code] || null;
}
