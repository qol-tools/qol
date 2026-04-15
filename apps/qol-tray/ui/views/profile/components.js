import { html } from '../../lib/html.js';
import { useCallback } from 'preact/hooks';
import { CodeBlock } from '../../lib/components/CodeBlock.js';
import { Modal, ModalFooter } from '../../lib/components/ModalPreact.js';
import { CustomSelect } from '../../lib/components/CustomSelect.js';
import { Badge } from '../../lib/components/StatusIndicators.js';
import { Button } from '../../lib/components/Button.js';
import { ToggleSwitch } from '../../lib/components/ToggleSwitch.js';
import { Surface } from '../../lib/components/Surface.js';
import { BackupRow } from '../../components/domain-rows/BackupRow.js';
import { toast } from '../../lib/toast.js';
import {
    FIELD_KIND_BOOLEAN,
    FIELD_KIND_PASSWORD,
    FIELD_KIND_SELECT,
    fieldHint,
    fieldLabels,
    fieldOptions,
    fieldPlaceholder,
    fieldValue,
    providerFieldInputId,
    providerFieldSurfaceId,
} from './form.js';
import {
    buildBadges,
    formatBackupPreview,
    formatBytes,
    importCounts,
    importSummary,
} from './summary.js';

function surfaceSel(ctrl, id) {
    const s = ctrl.surfaceById.get(id);
    if (!s) return null;
    return { index: s.index, selected: s.index === ctrl.selectedIndex, onSelect: ctrl.setSelectedIndex };
}

export function ProfileActionButton({ id, ctrl }) {
    const s = ctrl.surfaceById.get(id);
    const sel = surfaceSel(ctrl, id);
    if (!s || !sel) return null;
    return html`<${Button} variant=${s.variant || 'btn-ghost'} className="profile-action-btn"
        ...${sel} onActivate=${s.run}>${s.label}<//>`;
}

function ProfileInputField({ id, fieldId, label, hint = '', value, placeholder, type = 'text', className = '', ctrl, onInput }) {
    const sel = surfaceSel(ctrl, fieldId);
    if (!sel) return null;
    const cls = ['form-group', 'profile-input-surface', className].filter(Boolean).join(' ');
    return html`
        <${Surface} className=${cls} ...${sel}
            onActivate=${() => { const el = document.getElementById(id); if (el) { el.focus(); el.select?.(); } }}>
            <label for=${id}>${label}${hint && html`<span class="hint"> ${hint}</span>`}</label>
            <input id=${id} type=${type} class="profile-field-input" value=${value} placeholder=${placeholder} data-profile-editable="" onInput=${onInput} />
        <//>
    `;
}

function activateSelect(e) {
    if (e.target?.closest('.custom-select-trigger')) return;
    e.currentTarget?.querySelector('.custom-select-trigger')?.click();
}

export function ProfileSelectField({ label, hint = '', value, options, labels, className = '', ctrl, fieldId, onChange, compact = false }) {
    const sel = surfaceSel(ctrl, fieldId);
    if (!sel) return null;
    if (compact) {
        return html`<${Surface} className="profile-select-surface profile-select-compact" ...${sel} onActivate=${activateSelect}>
            <${CustomSelect} value=${value} options=${options} labels=${labels} onChange=${onChange} />
        <//>`;
    }
    const cls = ['form-group', 'profile-input-surface', 'profile-select-surface', className].filter(Boolean).join(' ');
    return html`
        <${Surface} className=${cls} ...${sel} onActivate=${activateSelect}>
            <label>${label}${hint && html`<span class="hint"> ${hint}</span>`}</label>
            <${CustomSelect} value=${value} options=${options} labels=${labels} onChange=${onChange} />
        <//>
    `;
}

export function ProfileToggleField({ label, checked, onChange, ctrl, fieldId }) {
    const sel = surfaceSel(ctrl, fieldId);
    if (!sel) return null;
    return html`<${ToggleSwitch} label=${label} checked=${checked} onChange=${onChange} ...${sel} />`;
}

export function renderProviderField({ field, form, syncStatus, ctrl, updateForm }) {
    if (!field) return null;
    const className = field.full_width ? 'profile-row-full' : '';
    const key = field.key;
    const fieldId = providerFieldSurfaceId(key);
    if (field.field_kind === FIELD_KIND_BOOLEAN) {
        return html`
            <${ProfileToggleField} key=${key}
                label=${field.label}
                checked=${form?.[key] ?? true}
                onChange=${(value) => updateForm(key, value)}
                ctrl=${ctrl} fieldId=${fieldId} />
        `;
    }
    if (field.field_kind === FIELD_KIND_SELECT) {
        const options = fieldOptions(field, form);
        return html`
            <${ProfileSelectField} key=${key}
                label=${field.label} hint=${field.hint || ''}
                value=${fieldValue(form, key)} options=${options} labels=${fieldLabels(field, options)}
                className=${className}
                ctrl=${ctrl} fieldId=${fieldId}
                onChange=${(value) => updateForm(key, value)} />
        `;
    }
    return html`
        <${ProfileInputField} key=${key}
            id=${providerFieldInputId(key)}
            type=${field.field_kind === FIELD_KIND_PASSWORD ? 'password' : 'text'}
            label=${field.label} hint=${fieldHint(field)}
            value=${fieldValue(form, key)} placeholder=${fieldPlaceholder(field, syncStatus)}
            className=${className}
            ctrl=${ctrl} fieldId=${fieldId}
            onInput=${(event) => updateForm(key, event.currentTarget.value)} />
    `;
}

export function ProfileBackupRow({ backup, incident, ctrl, onOpen }) {
    const sel = surfaceSel(ctrl, `backup:${backup.file_name}`);
    if (!sel) return null;
    const review = incident?.backup_file === backup.file_name;
    return html`<${BackupRow} time=${backup.created_at} fileName=${backup.file_name}
        size=${formatBytes(backup.size_bytes)} review=${review}
        ...${sel} onActivate=${onOpen} />`;
}

export function ImportFeedback({ lastImport }) {
    const counts = importCounts(lastImport.result);
    const badges = buildBadges(counts);

    return html`
        <div class="profile-status" data-variant=${lastImport.result.success ? 'success' : 'warning'}>
            <div class="profile-status-head">
                <div>
                    <div class="profile-status-title">${importSummary(lastImport.result)}</div>
                    <div class="profile-status-file">${lastImport.fileName}</div>
                </div>
            </div>
            <div class="profile-badge-row">
                ${badges.map(badge => html`
                    <${Badge} key=${badge.label} className=${`profile-badge ${badge.className}`}>${badge.label}<//>
                `)}
            </div>
            ${lastImport.result.plugins?.length > 0 && html`
                <div class="profile-result-list">
                    ${lastImport.result.plugins.map(plugin => html`
                        <div key=${plugin.id} class="profile-result-row" data-status=${plugin.status}>
                            <div class="profile-result-id">${plugin.id}</div>
                            <div class="profile-result-status">
                                <${Badge} className=${`profile-badge profile-badge-${plugin.status}`}>${plugin.status}<//>
                            </div>
                            <div class="profile-result-message">${plugin.message}</div>
                        </div>
                    `)}
                </div>
            `}
        </div>
    `;
}

export function BackupPreviewModal({ preview, incident, onAcknowledge, onClose }) {
    const isIncidentBackup = incident?.backup_file === preview.file_name;
    const copy = useCallback(() => {
        navigator.clipboard.writeText(preview.content);
        toast('success', 'Copied to clipboard');
    }, [preview]);
    const acknowledge = useCallback(() => {
        onAcknowledge?.();
        onClose();
    }, [onAcknowledge, onClose]);

    const actions = [
        { label: 'Close', kbd: 'Esc', onClick: onClose },
        { label: 'Copy', kbd: 'C', onClick: copy },
    ];
    if (isIncidentBackup) {
        actions.push({ label: 'Looks Good', kbd: 'Enter', variant: 'btn-primary', onClick: acknowledge });
    } else {
        actions[1].variant = 'btn-primary';
    }

    return html`
        <${Modal} open=${true} onClose=${onClose} size="xl" dismissOnBackdrop=${true} className="edit-modal">
            <div class="edit-modal-content" tabIndex="-1">
                <h3>${preview.file_name}</h3>
                <${CodeBlock}
                    text=${formatBackupPreview(preview.content)}
                    onCopy=${() => toast('success', 'Copied to clipboard')}
                />
                <${ModalFooter} actions=${actions} />
            </div>
        <//>
    `;
}

