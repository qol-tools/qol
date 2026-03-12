import { html } from '../../lib/html.js';
import { useCallback } from 'preact/hooks';
import { Modal, ModalActions } from '../../components/ModalPreact.js';

const ACTION_TYPES = [
    { value: 'open_url', label: 'Open URL' },
    { value: 'launch_app', label: 'Launch App' },
];

const APP_REF_TYPES = [
    { value: 'bundle_id', label: 'Bundle ID' },
    { value: 'path', label: 'Path' },
    { value: 'name', label: 'Name' },
];

export function ShortcutEditModal({ modal, onChange, onClose, onSave }) {
    const title = modal.editing ? 'Edit Shortcut' : 'Add Shortcut';
    const nameRef = useCallback((el) => { if (el) el.focus(); }, []);

    const set = (key, value) => onChange({ ...modal.shortcut, [key]: value });
    const setAction = (patch) => onChange({ ...modal.shortcut, action: { ...modal.shortcut.action, ...patch } });

    return html`
        <${Modal} open=${true} onClose=${onClose} className="edit-modal">
            <div class="edit-modal-content">
                <h3>${title}</h3>
                <${IdField} value=${modal.shortcut.id} disabled=${modal.editing}
                    onChange=${(v) => set('id', v)} />
                <${NameField} value=${modal.shortcut.name} onChange=${(v) => set('name', v)} inputRef=${nameRef} />
                <${ActionTypeField} value=${modal.shortcut.action.type}
                    onChange=${(v) => onTypeChange(modal.shortcut, v, onChange)} />
                <${ActionFields} action=${modal.shortcut.action} onChange=${setAction} />
                <${OptionsFields} shortcut=${modal.shortcut} onChange=${set} />
                <${ModalActions} onClose=${onClose} onSave=${onSave} cancelTabIndex="8" saveTabIndex="9" />
            </div>
        <//>
    `;
}

function onTypeChange(shortcut, type, onChange) {
    if (type === 'open_url') {
        onChange({ ...shortcut, action: { type: 'open_url', url: '' } });
        return;
    }
    onChange({ ...shortcut, action: { type: 'launch_app', app: { type: 'path', path: '' } } });
}

function IdField({ value, disabled, onChange }) {
    return html`
        <div class="form-group">
            <label>ID</label>
            <input type="text" id="shortcut-id" tabindex="1" value=${value}
                   disabled=${disabled} placeholder="my-shortcut"
                   onInput=${(e) => onChange(e.target.value)} />
        </div>
    `;
}

function NameField({ value, onChange, inputRef }) {
    return html`
        <div class="form-group">
            <label>Name</label>
            <input type="text" ref=${inputRef} tabindex="2" value=${value}
                   placeholder="My Shortcut"
                   onInput=${(e) => onChange(e.target.value)} />
        </div>
    `;
}

function ActionTypeField({ value, onChange }) {
    return html`
        <div class="form-group">
            <label>Type</label>
            <select tabindex="3" value=${value}
                    onChange=${(e) => onChange(e.target.value)}>
                ${ACTION_TYPES.map(t => html`<option key=${t.value} value=${t.value}>${t.label}</option>`)}
            </select>
        </div>
    `;
}

function ActionFields({ action, onChange }) {
    if (action.type === 'open_url') {
        return html`<fragment>
            <${UrlField} url=${action.url || ''} onChange=${(v) => onChange({ url: v })} />
            <${BrowserOverrideField} browser=${action.browser_override} onChange=${(v) => onChange({ browser_override: v || undefined })} />
        </fragment>`;
    }
    return html`<${AppRefField} app=${action.app} onChange=${(v) => onChange({ app: v })} />`;
}

function UrlField({ url, onChange }) {
    return html`
        <div class="form-group">
            <label>URL</label>
            <input type="text" tabindex="4" value=${url}
                   placeholder="https://example.com"
                   onInput=${(e) => onChange(e.target.value)} />
        </div>
    `;
}

function BrowserOverrideField({ browser, onChange }) {
    const hasOverride = !!browser;
    const refType = browser?.type || 'bundle_id';
    const refValue = browser ? (browser.id || browser.path || browser.name || '') : '';

    const toggle = () => {
        if (hasOverride) return onChange(null);
        onChange({ type: 'bundle_id', id: '' });
    };

    const setRef = (type, value) => {
        const key = type === 'bundle_id' ? 'id' : type;
        onChange({ type, [key]: value });
    };

    return html`
        <div class="form-group">
            <label>
                <input type="checkbox" checked=${hasOverride} onChange=${toggle} />
                ${' '}Browser override
            </label>
            ${hasOverride && html`
                <div class="form-group-inline">
                    <select value=${refType} onChange=${(e) => setRef(e.target.value, refValue)}>
                        ${APP_REF_TYPES.map(t => html`<option key=${t.value} value=${t.value}>${t.label}</option>`)}
                    </select>
                    <input type="text" tabindex="5" value=${refValue}
                           placeholder=${refType === 'bundle_id' ? 'com.google.Chrome' : refType === 'path' ? '/Applications/Firefox.app' : 'Firefox'}
                           onInput=${(e) => setRef(refType, e.target.value)} />
                </div>
            `}
        </div>
    `;
}

function AppRefField({ app, onChange }) {
    const refType = app?.type || 'path';
    const refValue = app ? (app.id || app.path || app.name || '') : '';

    const setRef = (type, value) => {
        const key = type === 'bundle_id' ? 'id' : type;
        onChange({ type, [key]: value });
    };

    return html`
        <div class="form-group">
            <label>Application</label>
            <div class="form-group-inline">
                <select tabindex="4" value=${refType} onChange=${(e) => setRef(e.target.value, refValue)}>
                    ${APP_REF_TYPES.map(t => html`<option key=${t.value} value=${t.value}>${t.label}</option>`)}
                </select>
                <input type="text" tabindex="5" value=${refValue}
                       placeholder=${refType === 'bundle_id' ? 'com.apple.Safari' : refType === 'path' ? '/Applications/App.app' : 'App Name'}
                       onInput=${(e) => setRef(refType, e.target.value)} />
            </div>
        </div>
    `;
}

function OptionsFields({ shortcut, onChange }) {
    return html`
        <div class="form-group form-group-row">
            <label>
                <input type="checkbox" checked=${shortcut.enabled}
                       onChange=${(e) => onChange('enabled', e.target.checked)} />
                ${' '}Enabled
            </label>
            <label>
                <input type="checkbox" checked=${shortcut.export_to_launcher}
                       onChange=${(e) => onChange('export_to_launcher', e.target.checked)} />
                ${' '}Export to launcher
            </label>
        </div>
    `;
}

