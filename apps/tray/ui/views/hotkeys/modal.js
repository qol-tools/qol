import { html } from '../../lib/html.js';
import { useMemo } from 'preact/hooks';
import { CustomSelect } from '../../lib/components/CustomSelect.js';
import { Surface } from '../../lib/components/Surface.js';
import { TextInput } from '../../lib/components/TextInput.js';

export function PluginSelect({ modal, plugins, onChange }) {
    const options = useMemo(() => plugins.map(p => p.uid), [plugins]);
    const labels = useMemo(() => Object.fromEntries(plugins.map(p => [p.uid, p.name])), [plugins]);
    return html`<${CustomSelect} value=${modal.pluginUid} options=${options} labels=${labels} onChange=${onChange} />`;
}

export function ActionSelect({ modal, onChange, disabled }) {
    const options = useMemo(() => modal.availableActions.map(a => a.id), [modal.availableActions]);
    const labels = useMemo(() => Object.fromEntries(modal.availableActions.map(a => [a.id, a.label])), [modal.availableActions]);

    if (disabled) {
        return html`
            <div class="custom-select">
                <${Surface} as="button" type="button" className="custom-select-trigger" disabled>
                    <span class="custom-select-value">All actions assigned</span>
                    <span class="custom-select-arrow">${'\u25BE'}</span>
                <//>
            </div>
        `;
    }
    return html`<${CustomSelect} value=${modal.action} options=${options} labels=${labels} onChange=${onChange} />`;
}

export function KeyInput({ modal, recording, onStartRecording, disabled }) {
    return html`
        <div class="key-input-row">
            <${TextInput} id="hotkey-key"
                   value=${modal.key} readonly disabled=${disabled}
                   className=${recording ? 'recording' : ''}
                   placeholder=${recording ? 'Press keys... (Esc to cancel)' : 'Press Enter to record'}
                   onClick=${!disabled ? onStartRecording : undefined} />
        </div>
    `;
}

export function createEditModalState(hotkey, keepPlugin, getAvailableActions) {
    const pluginUid = keepPlugin || hotkey?.plugin_uid || '';
    const availableActions = pluginUid ? getAvailableActions(pluginUid, hotkey?.id) : [];
    return {
        hotkey,
        pluginUid,
        action: hotkey?.action || availableActions[0]?.id || '',
        key: hotkey?.key || '',
        enabled: hotkey?.enabled !== false,
        availableActions
    };
}

export function changeEditModalPlugin(previous, pluginUid, getAvailableActions) {
    if (!previous) return previous;
    const availableActions = getAvailableActions(pluginUid, previous.hotkey?.id);
    return {
        ...previous,
        pluginUid,
        action: availableActions[0]?.id || '',
        availableActions
    };
}

