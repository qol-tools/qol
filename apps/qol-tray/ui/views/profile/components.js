import { html } from '../../lib/html.js';
import { useCallback } from 'preact/hooks';
import { CodeBlock } from '../../components/CodeBlock.js';
import { Modal, ModalFooter } from '../../components/ModalPreact.js';
import { CustomSelect } from '../plugin-config/fields/CustomSelect.js';
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
} from './form.js';
import { surfaceProps } from './use-surface-nav.js';
import {
    buildBadges,
    formatBackupPreview,
    formatBytes,
    importCounts,
    importSummary,
} from './summary.js';

export function ProfileActionButton({ surface, selectedIndex, setSelectedIndex }) {
    const sp = surfaceProps(surface, selectedIndex, setSelectedIndex);
    if (!sp) return null;
    const selected = surface.index === selectedIndex;
    const classes = ['btn', surface.variant || 'btn-ghost', 'profile-action-btn'];
    if (selected) classes.push('is-selected');
    return html`
        <button type="button" class=${classes.join(' ')} ...${sp}
            onClick=${() => { setSelectedIndex(surface.index); surface.run?.(); }}>
            ${surface.label}
        </button>
    `;
}

function ProfileInputField({ id, label, hint = '', value, placeholder, type = 'text', className = '', surface, selectedIndex, setSelectedIndex, onInput }) {
    const sp = surfaceProps(surface, selectedIndex, setSelectedIndex);
    if (!sp) return null;
    const classes = ['form-group', 'profile-input-surface', className].filter(Boolean).join(' ');
    return html`
        <div class=${classes} ...${sp}>
            <label for=${id}>${label}${hint && html`<span class="hint"> ${hint}</span>`}</label>
            <input id=${id} type=${type} class="profile-field-input" value=${value} placeholder=${placeholder} data-profile-editable="" onInput=${onInput} />
        </div>
    `;
}

export function ProfileSelectField({ label, hint = '', value, options, labels, className = '', surface, selectedIndex, setSelectedIndex, onChange, compact = false }) {
    const sp = surfaceProps(surface, selectedIndex, setSelectedIndex);
    if (!sp) return null;
    if (compact) {
        return html`<div class="profile-select-surface profile-select-compact" ...${sp}>
            <${CustomSelect} value=${value} options=${options} labels=${labels} onChange=${onChange} />
        </div>`;
    }
    const classes = ['form-group', 'profile-input-surface', 'profile-select-surface', className].filter(Boolean).join(' ');
    return html`
        <div class=${classes} ...${sp}>
            <label>${label}${hint && html`<span class="hint"> ${hint}</span>`}</label>
            <${CustomSelect} value=${value} options=${options} labels=${labels} onChange=${onChange} />
        </div>
    `;
}

export function ProfileToggleField({ label, checked, onChange, surface, selectedIndex, setSelectedIndex }) {
    const sp = surfaceProps(surface, selectedIndex, setSelectedIndex);
    if (!sp) return null;
    return html`
        <div class="toggle-inline profile-toggle-inline profile-toggle-surface" ...${sp} onClick=${() => onChange(!checked)}>
            <div class=${`toggle-track ${checked ? 'on' : ''} profile-toggle-track`} tabIndex="-1" role="switch" aria-checked=${checked}>
                <div class="toggle-thumb"></div>
            </div>
            <span class="toggle-inline-label">${label}</span>
        </div>
    `;
}

export function renderProviderField({
    field,
    form,
    syncStatus,
    selectedIndex,
    setSelectedIndex,
    surface,
    updateForm,
}) {
    if (!field) {
        return null;
    }
    const className = field.full_width ? 'profile-row-full' : '';
    const key = field.key;
    if (field.field_kind === FIELD_KIND_BOOLEAN) {
        return html`
            <${ProfileToggleField}
                key=${key}
                label=${field.label}
                checked=${form?.[key] ?? true}
                onChange=${(value) => updateForm(key, value)}
                surface=${surface}
                selectedIndex=${selectedIndex}
                setSelectedIndex=${setSelectedIndex}
            />
        `;
    }
    if (field.field_kind === FIELD_KIND_SELECT) {
        const options = fieldOptions(field, form);
        return html`
            <${ProfileSelectField}
                key=${key}
                label=${field.label}
                hint=${field.hint || ''}
                value=${fieldValue(form, key)}
                options=${options}
                labels=${fieldLabels(field, options)}
                className=${className}
                surface=${surface}
                selectedIndex=${selectedIndex}
                setSelectedIndex=${setSelectedIndex}
                onChange=${(value) => updateForm(key, value)}
            />
        `;
    }
    return html`
        <${ProfileInputField}
            key=${key}
            id=${providerFieldInputId(key)}
            type=${field.field_kind === FIELD_KIND_PASSWORD ? 'password' : 'text'}
            label=${field.label}
            hint=${fieldHint(field)}
            value=${fieldValue(form, key)}
            placeholder=${fieldPlaceholder(field, syncStatus)}
            className=${className}
            surface=${surface}
            selectedIndex=${selectedIndex}
            setSelectedIndex=${setSelectedIndex}
            onInput=${(event) => updateForm(key, event.currentTarget.value)}
        />
    `;
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
                    <span key=${badge.label} class=${`badge profile-badge ${badge.className}`}>${badge.label}</span>
                `)}
            </div>
            ${lastImport.result.plugins?.length > 0 && html`
                <div class="profile-result-list">
                    ${lastImport.result.plugins.map(plugin => html`
                        <div key=${plugin.id} class="profile-result-row" data-status=${plugin.status}>
                            <div class="profile-result-id">${plugin.id}</div>
                            <div class="profile-result-status">
                                <span class=${`badge profile-badge profile-badge-${plugin.status}`}>${plugin.status}</span>
                            </div>
                            <div class="profile-result-message">${plugin.message}</div>
                        </div>
                    `)}
                </div>
            `}
        </div>
    `;
}

export function ProfileBackupRow({ backup, incident, surface, selectedIndex, setSelectedIndex, onOpen }) {
    const sp = surfaceProps(surface, selectedIndex, setSelectedIndex);
    if (!sp) return null;
    const review = incident?.backup_file === backup.file_name;
    return html`
        <div class="profile-backup-row" role="listitem" ...${sp} onClick=${onOpen}>
            <div class="profile-backup-row-top">
                <span class="profile-backup-time">${backup.created_at}</span>
                ${review && html`<span class="badge profile-badge profile-badge-skipped">Review backup</span>`}
                <span class="profile-backup-size">${formatBytes(backup.size_bytes)}</span>
            </div>
            <div class="profile-backup-row-bottom">
                <span class="profile-backup-file" data-selected-text="">${backup.file_name}</span>
            </div>
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

export function ProfileStatusStrip({ syncStatus, surfaceById, selectedIndex, setSelectedIndex }) {
    const health = syncStatus?.health || 'not_configured';
    return html`
        <div class="profile-toolbar">
            <div class="profile-status-strip" data-health=${health}>
                <div class="profile-status-strip-main">
                    <span class="profile-health-dot" data-health=${health}></span>
                    <span class="profile-status-strip-text">${profileHealthLabel(syncStatus)}</span>
                </div>
                <div class="profile-status-strip-meta">
                    <span>${profileRemoteSummary(syncStatus)}</span>
                    <span class="profile-status-strip-sep">${'\u00b7'}</span>
                    <span>${profileLastSyncSummary(syncStatus)}</span>
                </div>
            </div>
            <div class="profile-toolbar-actions">
                <${ProfileActionButton} surface=${surfaceById.get('export')} selectedIndex=${selectedIndex} setSelectedIndex=${setSelectedIndex} />
                <${ProfileActionButton} surface=${surfaceById.get('import')} selectedIndex=${selectedIndex} setSelectedIndex=${setSelectedIndex} />
            </div>
        </div>
    `;
}
