import { html } from '../../lib/html.js';
import { useMemo } from 'preact/hooks';
import { CustomSelect } from '../../lib/components/CustomSelect.js';

export function PluginSelect({ modal, plugins, onChange }) {
    const options = useMemo(() => plugins.map(p => p.id), [plugins]);
    const labels = useMemo(() => Object.fromEntries(plugins.map(p => [p.id, p.name])), [plugins]);
    return html`<${CustomSelect} value=${modal.pluginId} options=${options} labels=${labels} onChange=${onChange} />`;
}

export function ActionSelect({ modal, onChange, disabled }) {
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

export function KeyInput({ modal, recording, onStartRecording, disabled }) {
    return html`
        <div class="key-input-row">
            <input type="text" id="hotkey-key"
                   value=${modal.key} readonly disabled=${disabled}
                   class=${recording ? 'recording' : ''}
                   placeholder=${recording ? 'Press keys... (Esc to cancel)' : 'Press Enter to record'}
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
        enabled: hotkey?.enabled !== false,
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

