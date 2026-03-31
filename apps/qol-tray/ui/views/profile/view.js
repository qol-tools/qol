import { html } from '../../lib/html.js';
import { useState } from 'preact/hooks';
import { PageHeader } from '../../components/PageHeader.js';
import { useRegisterCommands } from '../../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../../components/app/view-keyboard-context.js';
import { useProfileController } from './use-controller.js';
import {
    BackupPreviewModal,
    ImportFeedback,
    ProfileActionButton,
    ProfileBackupRow,
    ProfileSelectField,
    ProfileToggleField,
    renderProviderField,
} from './components.js';
import { providerFieldSurfaceId } from './form.js';
import { surfaceProps } from './use-surface-nav.js';
import {
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

    const health = ctrl.syncStatus?.health || 'not_configured';

    return html`
        <div id="profile-page" class="view-container content-shell profile-view-shell">
            <${PageHeader} title="Profile" subtitle="Cloud sync, import and export, and recovery for your QoL Tray setup" />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <div class="profile-page-stack">
                        <div id="profile-sections" class="content-frame profile-frame" data-surface-container="">
                            ${ctrl.lastImport && html`<${ImportFeedback} lastImport=${ctrl.lastImport} />`}

                            <section class="profile-section">
                                <!-- Status line -->
                                <div class="profile-status-line">
                                    <span class="profile-health-dot" data-health=${health}></span>
                                    <span class="profile-status-label">${profileHealthLabel(ctrl.syncStatus)}</span>
                                    ${ctrl.configured && html`
                                        <span class="profile-status-meta">${'\u00b7'} ${profileLastSyncSummary(ctrl.syncStatus)}</span>
                                    `}
                                </div>

                                <!-- Alerts -->
                                ${ctrl.syncStatus?.incident && html`
                                    <div class="profile-sync-alert" data-variant="warning">
                                        ${ctrl.syncStatus.incident.message}
                                    </div>
                                `}
                                ${ctrl.syncStatus?.last_error && html`
                                    <div class="profile-sync-alert" data-variant="error">
                                        ${ctrl.syncStatus.last_error}
                                    </div>
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
                                        <${ProfileActionButton} surface=${ctrl.surfaceById.get('pull')} selectedIndex=${ctrl.selectedIndex} setSelectedIndex=${ctrl.setSelectedIndex} />
                                        <${ProfileActionButton} surface=${ctrl.surfaceById.get('push')} selectedIndex=${ctrl.selectedIndex} setSelectedIndex=${ctrl.setSelectedIndex} />
                                        <${ProfileActionButton} surface=${ctrl.surfaceById.get('acknowledge')} selectedIndex=${ctrl.selectedIndex} setSelectedIndex=${ctrl.setSelectedIndex} />
                                        <span class="profile-actions-spacer"></span>
                                        <${ProfileActionButton} surface=${ctrl.surfaceById.get('disconnect')} selectedIndex=${ctrl.selectedIndex} setSelectedIndex=${ctrl.setSelectedIndex} />
                                    </div>
                                `}

                                <!-- Not connected: centered connect button -->
                                ${!ctrl.authPrompt && !ctrl.configured && html`
                                    <div class="profile-connect-row">
                                        <${ProfileActionButton} surface=${ctrl.surfaceById.get('connect')} selectedIndex=${ctrl.selectedIndex} setSelectedIndex=${ctrl.setSelectedIndex} />
                                    </div>
                                `}

                                <!-- Settings expander -->
                                ${!ctrl.authPrompt && html`
                                    <${SettingsExpander}
                                        open=${showSettings}
                                        onToggle=${() => setShowSettings(!showSettings)}
                                        surface=${ctrl.surfaceById.get('settings')}
                                        selectedIndex=${ctrl.selectedIndex}
                                        setSelectedIndex=${ctrl.setSelectedIndex}
                                    >
                                        <div class="btn-expander-trigger">
                                            <span class="btn-icon btn-icon-chevron">${'\u25b6'}</span>
                                            Settings
                                        </div>
                                        <div class="btn-expander-body" onClick=${(e) => e.stopPropagation()}>
                                            <div class="profile-settings-field">
                                                <span class="profile-settings-label">Provider</span>
                                                <${ProfileSelectField}
                                                    value=${ctrl.form.provider}
                                                    options=${ctrl.providerOptions.map(p => p.kind)}
                                                    labels=${ctrl.providerLabels}
                                                    surface=${ctrl.surfaceById.get('provider')}
                                                    selectedIndex=${ctrl.selectedIndex}
                                                    setSelectedIndex=${ctrl.setSelectedIndex}
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
                                                    selectedIndex: ctrl.selectedIndex,
                                                    setSelectedIndex: ctrl.setSelectedIndex,
                                                    surface: ctrl.surfaceById.get(providerFieldSurfaceId(field.key)),
                                                    updateForm: ctrl.updateForm,
                                                })
                                            )}
                                        </div>
                                    <//>
                                `}
                            </section>

                            <section class="profile-section">
                                <div class="section-header">
                                    <h2>Backups</h2>
                                    <div class="section-actions">
                                        ${ctrl.backups.length > 0 && html`
                                            <span class="badge profile-backup-count">${ctrl.backups.length}</span>
                                        `}
                                        ${ctrl.incident?.backup_file && html`
                                            <span class="badge profile-badge profile-badge-skipped">${ctrl.incident.backup_file}</span>
                                        `}
                                        <${ProfileActionButton} surface=${ctrl.surfaceById.get('export')} selectedIndex=${ctrl.selectedIndex} setSelectedIndex=${ctrl.setSelectedIndex} />
                                        <${ProfileActionButton} surface=${ctrl.surfaceById.get('import')} selectedIndex=${ctrl.selectedIndex} setSelectedIndex=${ctrl.setSelectedIndex} />
                                        <${ProfileActionButton} surface=${ctrl.surfaceById.get('open-backups')} selectedIndex=${ctrl.selectedIndex} setSelectedIndex=${ctrl.setSelectedIndex} />
                                    </div>
                                </div>
                                ${ctrl.backups.length > 0 && html`
                                    <div class="profile-backup-list" role="list">
                                        ${ctrl.backups.map(backup => html`
                                            <${ProfileBackupRow}
                                                key=${backup.file_name}
                                                backup=${backup}
                                                incident=${ctrl.incident}
                                                surface=${ctrl.surfaceById.get(`backup:${backup.file_name}`)}
                                                selectedIndex=${ctrl.selectedIndex}
                                                setSelectedIndex=${ctrl.setSelectedIndex}
                                                onOpen=${() => ctrl.handlePreviewBackup(backup.file_name)}
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
                        </div>
                    </div>
                </div>
            </div>
        </div>
        ${ctrl.backupPreview && html`<${BackupPreviewModal}
            preview=${ctrl.backupPreview}
            incident=${ctrl.incident}
            onAcknowledge=${ctrl.handleAcknowledge}
            onClose=${() => ctrl.setBackupPreview(null)}
        />`}
    `;
}

function renderSettingsField({ field, form, syncStatus, configured, selectedIndex, setSelectedIndex, surface, updateForm }) {
    const configuredValue = syncStatus?.[field.key];
    const sameProvider = form?.provider === syncStatus?.provider;

    // Field has a configured value from the active connection → read-only info
    if (configured && sameProvider && configuredValue) {
        return html`<${SettingsInfoField}
            key=${field.key}
            label=${field.label}
            surface=${surface}
            selectedIndex=${selectedIndex}
            setSelectedIndex=${setSelectedIndex}
        >${fieldInfoValue(field.key, configuredValue)}<//>`;
    }

    // Otherwise → editable input
    return renderProviderField({ field, form, syncStatus, selectedIndex, setSelectedIndex, surface, updateForm });
}

function fieldInfoValue(key, value) {
    if (key === 'gist_id') {
        const url = `https://gist.github.com/${value}`;
        return html`<a href=${url} class="profile-settings-link" target="_blank" onClick=${(e) => e.stopPropagation()}>${value} ${'\u2197'}</a>`;
    }
    return value;
}

function SettingsInfoField({ label, surface, selectedIndex, setSelectedIndex, children }) {
    const sp = surfaceProps(surface, selectedIndex, setSelectedIndex);
    if (!sp) return null;
    return html`
        <div class="profile-settings-field" ...${sp}>
            <span class="profile-settings-label">${label}</span>
            <span class="profile-settings-value">${children}</span>
        </div>
    `;
}

function SettingsExpander({ open, onToggle, surface, selectedIndex, setSelectedIndex, children }) {
    const sp = surfaceProps(surface, selectedIndex, setSelectedIndex);
    if (!sp) return null;
    return html`
        <div class="btn btn-ghost btn-expander" ...${sp}
            aria-expanded=${open ? 'true' : 'false'}
            onClick=${(e) => {
                if (e.target.closest('.btn-expander-body')) return;
                setSelectedIndex(surface.index);
                onToggle();
            }}>
            ${children}
        </div>
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
            <button class="btn btn-primary" onClick=${onOpenGitHub}>
                Open GitHub
            </button>
        </div>
    `;
}
