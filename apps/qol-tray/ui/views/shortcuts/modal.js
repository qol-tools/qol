import { html } from '../../lib/html.js';
import { ToggleSwitch } from '../../lib/components/ToggleSwitch.js';
import { CustomSelect } from '../../lib/components/CustomSelect.js';
import { ModalActions } from '../../lib/components/ModalPreact.js';

const ACTION_TYPES = [
    { value: 'open_url', label: 'Open URL' },
    { value: 'launch_app', label: 'Launch App' },
];
const ACTION_TYPE_OPTIONS = ACTION_TYPES.map(t => t.value);
const ACTION_TYPE_LABELS = Object.fromEntries(ACTION_TYPES.map(t => [t.value, t.label]));

const APP_REF_TYPES = [
    { value: 'bundle_id', label: 'Bundle ID' },
    { value: 'path', label: 'Path' },
    { value: 'name', label: 'Name' },
];

const APP_REF_OPTIONS = APP_REF_TYPES.map(t => t.value);
const APP_REF_LABELS = Object.fromEntries(APP_REF_TYPES.map(t => [t.value, t.label]));

function refKey(type) {
    return type === 'bundle_id' ? 'id' : type;
}

function extractRefValue(ref) {
    if (!ref) return '';
    return ref.id || ref.path || ref.name || '';
}

export function ShortcutEditForm({ modal, fieldProps, onChange, onClose, onSave }) {
    const set = (key, value) => onChange({ ...modal.shortcut, [key]: value });
    const setAction = (patch) => onChange({ ...modal.shortcut, action: { ...modal.shortcut.action, ...patch } });
    const isUrl = modal.shortcut.action.type === 'open_url';
    const canSave = computeCanSave(modal.shortcut);
    let fi = 0;

    return html`
        <div class="edit-modal-content">
            <${NameField} value=${modal.shortcut.name} onChange=${(v) => set('name', v)} fp=${fieldProps(fi++)} />
            <${ActionTypeField} value=${modal.shortcut.action.type}
                onChange=${(v) => onTypeChange(modal.shortcut, v, onChange)} fp=${fieldProps(fi++)} />
            ${isUrl && html`
                <${UrlField} url=${modal.shortcut.action.url || ''} onChange=${(v) => setAction({ url: v })} fp=${fieldProps(fi++)} />
                <${BrowserOverrideToggle} browser=${modal.shortcut.action.browser_override} onChange=${(v) => setAction({ browser_override: v || undefined })} fp=${fieldProps(fi++)} />
                ${modal.shortcut.action.browser_override && html`
                    <div class="form-group-children">
                        <${BrowserOverrideType} browser=${modal.shortcut.action.browser_override} onChange=${(v) => setAction({ browser_override: v || undefined })} fp=${fieldProps(fi++)} />
                        <${BrowserOverrideValue} browser=${modal.shortcut.action.browser_override} onChange=${(v) => setAction({ browser_override: v || undefined })} fp=${fieldProps(fi++)} />
                    </div>
                `}
            `}
            ${!isUrl && html`
                <${AppRefType} app=${modal.shortcut.action.app} onChange=${(v) => setAction({ app: v })} fp=${fieldProps(fi++)} />
                <${AppRefValue} app=${modal.shortcut.action.app} onChange=${(v) => setAction({ app: v })} fp=${fieldProps(fi++)} />
            `}
            <${OptionsFields} shortcut=${modal.shortcut} onChange=${set} fp1=${fieldProps(fi++)} fp2=${fieldProps(fi++)} />
            <${ModalActions} onClose=${onClose} onSave=${onSave} disabled=${!canSave} />
        </div>
    `;
}

export function computeCanSave(shortcut) {
    if (!shortcut) return false;
    if (!shortcut.name || !shortcut.name.trim()) return false;
    const action = shortcut.action;
    if (!action) return false;
    if (action.type === 'open_url') return !!(action.url && action.url.trim());
    if (action.type === 'launch_app') return !!extractRefValue(action.app).trim();
    return false;
}

function onTypeChange(shortcut, type, onChange) {
    if (type === 'open_url') {
        onChange({ ...shortcut, action: { type: 'open_url', url: '' } });
        return;
    }
    onChange({ ...shortcut, action: { type: 'launch_app', app: { type: 'path', path: '' } } });
}

function NameField({ value, onChange, fp }) {
    return html`
        <div class="form-group" ...${fp}>
            <label>Name</label>
            <input type="text" value=${value}
                   placeholder="My Shortcut"
                   onInput=${(e) => onChange(e.target.value)} />
        </div>
    `;
}

function ActionTypeField({ value, onChange, fp }) {
    return html`
        <div class="form-group" ...${fp}>
            <label>Type</label>
            <${CustomSelect} value=${value} options=${ACTION_TYPE_OPTIONS} labels=${ACTION_TYPE_LABELS} onChange=${onChange} />
        </div>
    `;
}

function UrlField({ url, onChange, fp }) {
    return html`
        <div class="form-group" ...${fp}>
            <label>URL</label>
            <input type="text" value=${url}
                   placeholder="https://example.com"
                   onInput=${(e) => onChange(e.target.value)} />
        </div>
    `;
}

function BrowserOverrideToggle({ browser, onChange, fp }) {
    const toggle = () => {
        if (browser) return onChange(null);
        onChange({ type: 'bundle_id', id: '' });
    };
    return html`
        <div class="form-group" ...${fp}>
            <${ToggleSwitch} checked=${!!browser} onChange=${toggle} label="Browser override" />
        </div>
    `;
}

function BrowserOverrideType({ browser, onChange, fp }) {
    const type = browser?.type || 'bundle_id';
    const value = extractRefValue(browser);
    return html`
        <div class="form-group" ...${fp}>
            <label>Browser type</label>
            <${CustomSelect} value=${type} options=${APP_REF_OPTIONS} labels=${APP_REF_LABELS}
                onChange=${(t) => onChange({ type: t, [refKey(t)]: value })} />
        </div>
    `;
}

function BrowserOverrideValue({ browser, onChange, fp }) {
    const type = browser?.type || 'bundle_id';
    return html`
        <div class="form-group" ...${fp}>
            <label>Browser value</label>
            <input type="text" value=${extractRefValue(browser)}
                   placeholder=${type === 'bundle_id' ? 'com.google.Chrome' : type === 'path' ? '/Applications/Firefox.app' : 'Firefox'}
                   onInput=${(e) => onChange({ type, [refKey(type)]: e.target.value })} />
        </div>
    `;
}

function AppRefType({ app, onChange, fp }) {
    const type = app?.type || 'path';
    const value = extractRefValue(app);
    return html`
        <div class="form-group" ...${fp}>
            <label>App type</label>
            <${CustomSelect} value=${type} options=${APP_REF_OPTIONS} labels=${APP_REF_LABELS}
                onChange=${(t) => onChange({ type: t, [refKey(t)]: value })} />
        </div>
    `;
}

function AppRefValue({ app, onChange, fp }) {
    const type = app?.type || 'path';
    return html`
        <div class="form-group" ...${fp}>
            <label>App value</label>
            <input type="text" value=${extractRefValue(app)}
                   placeholder=${type === 'bundle_id' ? 'com.apple.Safari' : type === 'path' ? '/Applications/App.app' : 'App Name'}
                   onInput=${(e) => onChange({ type, [refKey(type)]: e.target.value })} />
        </div>
    `;
}

function OptionsFields({ shortcut, onChange, fp1, fp2 }) {
    return html`
        <div class="form-group" ...${fp1}>
            <${ToggleSwitch} checked=${shortcut.enabled}
                onChange=${(v) => onChange('enabled', v)} label="Enabled" />
        </div>
        <div class="form-group" ...${fp2}>
            <${ToggleSwitch} checked=${shortcut.export_to_launcher}
                onChange=${(v) => onChange('export_to_launcher', v)} label="Export to launcher" />
        </div>
    `;
}
