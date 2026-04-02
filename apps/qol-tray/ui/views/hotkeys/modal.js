import { html } from '../../lib/html.js';
import { useMemo } from 'preact/hooks';
import { Modal, ModalActions } from '../../components/ModalPreact.js';
import { CustomSelect } from '../../components/CustomSelect.js';

const MODIFIER_KEYS = ['Control', 'Alt', 'Shift', 'Meta'];
const MODIFIER_NAMES = ['Ctrl', 'Alt', 'Shift', 'Super'];

const NAV_KEY_MAP = {
    Space: 'Space', Enter: 'Enter', Escape: 'Escape', Tab: 'Tab',
    Backspace: 'Backspace', Delete: 'Delete', Insert: 'Insert',
    Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown',
    ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right',
    F1: 'F1', F2: 'F2', F3: 'F3', F4: 'F4', F5: 'F5', F6: 'F6',
    F7: 'F7', F8: 'F8', F9: 'F9', F10: 'F10', F11: 'F11', F12: 'F12',
    PrintScreen: 'PrintScreen', Pause: 'Pause'
};

export function HotkeyEditModal({ modal, plugins, fieldProps, onPluginChange, onActionChange, onStartRecording, onClose, onSave }) {
    const title = modal.hotkey ? 'Edit Hotkey' : 'Add Hotkey';
    const exhausted = modal.availableActions.length === 0;

    return html`
        <${Modal} open=${true} onClose=${onClose} className="edit-modal">
            <div class="edit-modal-content">
                <h3>${title}</h3>
                <div class="form-group" ...${fieldProps(0)}>
                    <label>Plugin</label>
                    <${PluginSelect} modal=${modal} plugins=${plugins} onChange=${onPluginChange} />
                </div>
                <div class="form-group" ...${fieldProps(1)}>
                    <label>Action</label>
                    <${ActionSelect} modal=${modal} onChange=${onActionChange} disabled=${exhausted} />
                </div>
                <div class="form-group" ...${fieldProps(2)}>
                    <label>Shortcut <span class="hint">(Enter to record)</span></label>
                    <${KeyInput} modal=${modal} onStartRecording=${onStartRecording} disabled=${exhausted} />
                </div>
                <${ModalActions} onClose=${onClose} onSave=${onSave} disabled=${exhausted} />
            </div>
        <//>
    `;
}

function PluginSelect({ modal, plugins, onChange }) {
    const options = useMemo(() => plugins.map(p => p.id), [plugins]);
    const labels = useMemo(() => Object.fromEntries(plugins.map(p => [p.id, p.name])), [plugins]);
    return html`<${CustomSelect} value=${modal.pluginId} options=${options} labels=${labels} onChange=${onChange} />`;
}

function ActionSelect({ modal, onChange, disabled }) {
    const options = useMemo(() => modal.availableActions.map(a => a.id), [modal.availableActions]);
    const labels = useMemo(() => Object.fromEntries(modal.availableActions.map(a => [a.id, a.label])), [modal.availableActions]);

    if (disabled) {
        return html`
            <div class="custom-select">
                <button type="button" class="custom-select-trigger" disabled>
                    <span class="custom-select-value" style="opacity: 0.5">All actions assigned</span>
                    <span class="custom-select-arrow">${'\u25BE'}</span>
                </button>
            </div>
        `;
    }
    return html`<${CustomSelect} value=${modal.action} options=${options} labels=${labels} onChange=${onChange} />`;
}

function KeyInput({ modal, onStartRecording, disabled }) {
    return html`
        <div class="key-input-row">
            <input type="text" id="hotkey-key"
                   value=${modal.key} readonly disabled=${disabled}
                   class=${modal.recording ? 'recording' : ''}
                   placeholder=${modal.recording ? 'Press keys... (Esc to cancel)' : 'Press Enter to record'}
                   onClick=${!disabled ? onStartRecording : undefined} />
        </div>
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
    if (!previous) return previous;
    const availableActions = getAvailableActions(pluginId, previous.hotkey?.id);
    return {
        ...previous,
        pluginId,
        action: availableActions[0]?.id || '',
        availableActions
    };
}

export function applyRecordingKey(modal, event) {
    if (event.key === 'Escape') {
        return { modal: { ...modal, recording: false }, advance: false };
    }
    if (MODIFIER_KEYS.includes(event.key)) {
        const key = formatKeyEvent(event);
        return { modal: key ? { ...modal, key } : modal, advance: false };
    }

    const key = formatKeyEvent(event);
    if (!key || MODIFIER_NAMES.includes(key)) {
        return { modal, advance: false };
    }

    return {
        modal: { ...modal, key, recording: false },
        advance: true
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
    return NAV_KEY_MAP[code] || null;
}
