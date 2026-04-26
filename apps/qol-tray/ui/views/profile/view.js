import { html } from '../../lib/html.js';
import { useEffect, useState } from 'preact/hooks';
import { PageHeader } from '../../components/PageHeader.js';
import { SurfaceContainer } from '../../lib/components/SurfaceContainer.js';
import { Surface } from '../../lib/components/Surface.js';
import { Expander, ExpanderTrigger, ExpanderBody } from '../../lib/components/Expander.js';
import { Badge, HealthDot, Alert } from '../../lib/components/StatusIndicators.js';
import { Button } from '../../lib/components/Button.js';
import { CodeBlock } from '../../lib/components/CodeBlock.js';
import { useRegisterCommands } from '../../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../../app/view-keyboard-context.js';
import { toast } from '../../lib/toast.js';
import { useProfileController } from './use-controller.js';
import {
    ImportFeedback,
    ProfileActionButton,
    ProfileBackupRow,
    ProfileSelectField,
    ProfileToggleField,
    renderProviderField,
} from './components.js';
import { providerFieldSurfaceId } from './form.js';
import { backupPreviewSlot } from './use-backups.js';
import { openProfileBackupFile } from './actions.js';
import {
    formatBackupPreview,
    profileHealthLabel,
    profileLastSyncSummary,
} from './summary.js';

export function ProfileView({ syncStatus, syncProviders, onSyncStatusChange, refreshSyncStatus }) {
    const ctrl = useProfileController({
        syncStatus,
        syncProviders,
        onSyncStatusChange,
        refreshSyncStatus,
    });
    useRegisterViewKeyboard('profile', ctrl.handleKey, ctrl.isBlocking);
    useRegisterCommands('profile', ctrl.commands);
    const [showSettings, setShowSettings] = useState(false);

    useEffect(() => {
        backupPreviewSlot.set({
            preview: ctrl.backupPreview,
            incident: ctrl.incident,
            onAcknowledge: ctrl.handleAcknowledge,
        });
    }, [ctrl.backupPreview, ctrl.incident, ctrl.handleAcknowledge]);

    const health = ctrl.syncStatus?.health || 'not_configured';
    const settingsSurface = ctrl.surfaceById.get('settings');

    return html`
        <div id="profile-page" class="view-container content-shell profile-view-shell">
            <${PageHeader} subtitle="Cloud sync, import and export, and recovery for your QoL Tray setup" />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <div class="profile-page-stack">
                        <${SurfaceContainer} id="profile-sections" className="content-frame profile-frame">
                            ${ctrl.lastImport && html`<${ImportFeedback} lastImport=${ctrl.lastImport} />`}

                            <section class="profile-section">
                                <!-- Status line -->
                                <div class="profile-status-line">
                                    <${HealthDot} health=${health} />
                                    <span class="profile-status-label">${profileHealthLabel(ctrl.syncStatus)}</span>
                                    ${ctrl.configured && html`
                                        <span class="profile-status-meta">${'\u00b7'} ${profileLastSyncSummary(ctrl.syncStatus)}</span>
                                    `}
                                </div>

                                <!-- Alerts -->
                                ${ctrl.syncStatus?.incident && html`
                                    <${Alert} variant="warning">${ctrl.syncStatus.incident.message}<//>
                                `}
                                ${ctrl.syncStatus?.last_error && html`
                                    <${Alert} variant="error">${ctrl.syncStatus.last_error}<//>
                                `}

                                <!-- Device code flow -->
                                ${ctrl.authPrompt && html`
                                    <${DeviceCodePrompt}
                                        userCode=${ctrl.authPrompt.userCode}
                                        copied=${ctrl.authPrompt.copied}
                                        onOpenGitHub=${ctrl.openAuthLink}
                                    />
                                `}

                                <!-- Connected: action buttons -->
                                ${!ctrl.authPrompt && ctrl.configured && html`
                                    <div class="profile-actions-row">
                                        <${ProfileActionButton} id="pull" ctrl=${ctrl} />
                                        <${ProfileActionButton} id="push" ctrl=${ctrl} />
                                        <${ProfileActionButton} id="acknowledge" ctrl=${ctrl} />
                                        <span class="profile-actions-spacer"></span>
                                        <${ProfileActionButton} id="disconnect" ctrl=${ctrl} />
                                    </div>
                                `}

                                <!-- Not connected: centered connect button -->
                                ${!ctrl.authPrompt && !ctrl.configured && html`
                                    <div class="profile-connect-row">
                                        <${ProfileActionButton} id="connect" ctrl=${ctrl} />
                                    </div>
                                `}

                                <!-- Settings expander -->
                                ${!ctrl.authPrompt && html`
                                    <${Expander}
                                        open=${showSettings}
                                        onToggle=${() => { ctrl.setSelectedIndex(settingsSurface?.index); setShowSettings(!showSettings); }}
                                        index=${settingsSurface?.index}
                                        selected=${settingsSurface?.index === ctrl.selectedIndex}
                                        onSelect=${ctrl.setSelectedIndex}
                                    >
                                        <${ExpanderTrigger}>Settings<//>
                                        <${ExpanderBody}>
                                            <div class="profile-settings-field">
                                                <span class="profile-settings-label">Provider</span>
                                                <${ProfileSelectField}
                                                    value=${ctrl.form.provider}
                                                    options=${ctrl.providerOptions.map(p => p.kind)}
                                                    labels=${ctrl.providerLabels}
                                                    ctrl=${ctrl} fieldId="provider"
                                                    onChange=${(value) => ctrl.updateForm('provider', value)}
                                                    compact=${true}
                                                />
                                            </div>
                                            ${[...ctrl.basicProviderFields, ...ctrl.advancedProviderFields].map(field =>
                                                renderSettingsField({
                                                    field,
                                                    form: ctrl.form,
                                                    syncStatus: ctrl.syncStatus,
                                                    configured: ctrl.configured,
                                                    ctrl,
                                                    updateForm: ctrl.updateForm,
                                                })
                                            )}
                                        <//>
                                    <//>
                                `}
                            </section>

                            <section class="profile-section">
                                <div class="section-header">
                                    <h2>Backups</h2>
                                    <div class="section-actions">
                                        ${ctrl.backups.length > 0 && html`
                                            <${Badge} className="profile-backup-count">${ctrl.backups.length}<//>
                                        `}
                                        ${ctrl.incident?.backup_file && html`
                                            <${Badge} className="profile-badge profile-badge-skipped">${ctrl.incident.backup_file}<//>
                                        `}
                                        <${ProfileActionButton} id="export" ctrl=${ctrl} />
                                        <${ProfileActionButton} id="import" ctrl=${ctrl} />
                                        <${ProfileActionButton} id="open-backups" ctrl=${ctrl} />
                                    </div>
                                </div>
                                ${ctrl.backups.length > 0 && html`
                                    <div class="profile-backup-list" role="list">
                                        ${ctrl.backups.map(backup => html`
                                            <${ProfileBackupRow}
                                                key=${backup.file_name}
                                                backup=${backup}
                                                incident=${ctrl.incident}
                                                ctrl=${ctrl}
                                                onOpen=${() => ctrl.handlePreviewBackup(backup.file_name)}
                                                onOpenExternal=${ctrl.handleOpenBackupFile}
                                            />
                                        `)}
                                    </div>
                                `}
                                ${ctrl.backups.length === 0 && html`
                                    <p class="profile-empty">
                                        No sync backups yet. Recovery backups will appear here when remote state replaces local changes.
                                    </p>
                                `}
                            </section>
                        <//>
                    </div>
                </div>
            </div>
        </div>
    `;
}

function dispatchEscape() {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
}

export function BackupDetailSubPage() {
    const [, bump] = useState(0);
    useEffect(() => backupPreviewSlot.subscribe(() => bump(t => t + 1)), []);

    const { preview, incident, onAcknowledge } = backupPreviewSlot.get();
    if (!preview) {
        return html`<div class="view-container content-shell">
            <${PageHeader} title="Backup Preview" subtitle="Select a backup to view" />
        </div>`;
    }
    const isIncidentBackup = incident?.backup_file === preview.file_name;
    const copy = () => {
        navigator.clipboard.writeText(preview.content);
        toast('success', 'Copied to clipboard');
    };
    const acknowledge = () => { onAcknowledge?.(); dispatchEscape(); };
    return html`
        <div class="view-container content-shell">
            <${PageHeader} title=${preview.file_name} subtitle=${isIncidentBackup ? 'Backup awaiting review' : 'Backup preview'} />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame backup-detail-frame">
                        <${CodeBlock}
                            text=${formatBackupPreview(preview.content)}
                            onSecondaryActivate=${() => {
                                openProfileBackupFile(preview.file_name)
                                    .catch((err) => toast('error', `Failed to open: ${err.message}`));
                            }}
                            secondaryLabel="Open in editor"
                        />
                        <div class="backup-detail-actions">
                            <${Button} variant="btn-ghost" onActivate=${dispatchEscape}>Close <kbd>Esc</kbd><//>
                            <${Button} variant=${isIncidentBackup ? 'btn-ghost' : 'btn-primary'} onActivate=${copy}>Copy<//>
                            ${isIncidentBackup && html`<${Button} variant="btn-primary" onActivate=${acknowledge}>Looks Good<//>`}
                        </div>
                    <//>
                </div>
            </div>
        </div>
    `;
}

function renderSettingsField({ field, form, syncStatus, configured, ctrl, updateForm }) {
    const configuredValue = syncStatus?.[field.key];
    const sameProvider = form?.provider === syncStatus?.provider;

    if (configured && sameProvider && configuredValue) {
        const sel = ctrl.surfaceById.get(providerFieldSurfaceId(field.key));
        return html`<${SettingsInfoField} key=${field.key} label=${field.label}
            index=${sel?.index} selected=${sel?.index === ctrl.selectedIndex} onSelect=${ctrl.setSelectedIndex}
        >${fieldInfoValue(field.key, configuredValue)}<//>`;
    }

    return renderProviderField({ field, form, syncStatus, ctrl, updateForm });
}

function fieldInfoValue(key, value) {
    if (key === 'gist_id') {
        const url = `https://gist.github.com/${value}`;
        return html`<a href=${url} class="profile-settings-link" target="_blank" onClick=${(e) => e.stopPropagation()}>${value} ${'\u2197'}</a>`;
    }
    return value;
}

function SettingsInfoField({ label, index, selected, onSelect, children }) {
    return html`
        <${Surface} className="profile-settings-field" index=${index} selected=${selected} onSelect=${onSelect}>
            <span class="profile-settings-label">${label}</span>
            <span class="profile-settings-value">${children}</span>
        <//>
    `;
}

function DeviceCodePrompt({ userCode, copied, onOpenGitHub }) {
    const [justCopied, setJustCopied] = useState(copied);

    const handleCopy = async () => {
        try {
            await navigator.clipboard.writeText(userCode);
            setJustCopied(true);
            setTimeout(() => setJustCopied(false), 2000);
        } catch (_) {}
    };

    return html`
        <div class="profile-device-auth">
            <div class="profile-device-code" onClick=${handleCopy} title="Click to copy">
                ${userCode}
            </div>
            <p class="profile-device-hint">
                ${justCopied ? 'Copied!' : 'Click code to copy'} — then paste it on GitHub
            </p>
            <${Button} variant="btn-primary" onActivate=${onOpenGitHub}>Open GitHub<//>
        </div>
    `;
}
