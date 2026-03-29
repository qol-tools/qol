import { html } from '../../lib/html.js';
import { PageHeader } from '../../components/PageHeader.js';
import { useRegisterCommands } from '../../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../../components/app/view-keyboard-context.js';
import { useProfileController } from './use-controller.js';
import {
    BackupPreviewModal,
    ImportFeedback,
    ProfileActionButton,
    ProfileBackupRow,
    ProfileStatusStrip,
    ProfileSelectField,
    ProfileToggleField,
    renderProviderField,
} from './components.js';
import { providerFieldSurfaceId } from './form.js';

export function ProfileView({ syncStatus, syncProviders, onSyncStatusChange, refreshSyncStatus }) {
    const ctrl = useProfileController({
        syncStatus,
        syncProviders,
        onSyncStatusChange,
        refreshSyncStatus,
    });
    useRegisterViewKeyboard('profile', ctrl.handleKey, ctrl.isBlocking);
    useRegisterCommands('profile', ctrl.commands);

    return html`
        <div id="profile-page" class="view-container content-shell profile-view-shell">
            <${PageHeader} title="Profile" subtitle="Cloud sync, import and export, and recovery for your QoL Tray setup" />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <div class="profile-page-stack">
                        <div id="profile-sections" class="content-frame profile-frame">
                            <${ProfileStatusStrip}
                                syncStatus=${ctrl.syncStatus}
                                surfaceById=${ctrl.surfaceById}
                                selectedIndex=${ctrl.selectedIndex}
                                setSelectedIndex=${ctrl.setSelectedIndex}
                            />
                            ${ctrl.lastImport && html`<${ImportFeedback} lastImport=${ctrl.lastImport} />`}
                            <section class="profile-section">
                                <div class="section-header">
                                    <h2>Cloud Sync</h2>
                                </div>
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
                                <div class="profile-sync-grid">
                                    <${ProfileSelectField}
                                        label="Target"
                                        value=${ctrl.form.provider}
                                        options=${ctrl.providerOptions.map(provider => provider.kind)}
                                        labels=${ctrl.providerLabels}
                                        className="profile-row-full"
                                        surface=${ctrl.surfaceById.get('provider')}
                                        selectedIndex=${ctrl.selectedIndex}
                                        setSelectedIndex=${ctrl.setSelectedIndex}
                                        onChange=${(value) => ctrl.updateForm('provider', value)}
                                    />
                                    ${ctrl.basicProviderFields.map(field => renderProviderField({
                                        field,
                                        form: ctrl.form,
                                        syncStatus: ctrl.syncStatus,
                                        branchOptions: ctrl.branchOptions,
                                        selectedIndex: ctrl.selectedIndex,
                                        setSelectedIndex: ctrl.setSelectedIndex,
                                        surface: ctrl.surfaceById.get(providerFieldSurfaceId(field.key)),
                                        updateForm: ctrl.updateForm,
                                    }))}
                                </div>
                                <div class="profile-subsection">
                                    <div class="profile-subsection-header">
                                        <div class="profile-subsection-label">Advanced</div>
                                        <div class="profile-inline-options">
                                            <div class="profile-toggle-row">
                                                <${ProfileToggleField}
                                                    label="Pull on launch"
                                                    checked=${ctrl.form.pull_on_launch}
                                                    onChange=${(value) => ctrl.updateForm('pull_on_launch', value)}
                                                    surface=${ctrl.surfaceById.get('pull-on-launch')}
                                                    selectedIndex=${ctrl.selectedIndex}
                                                    setSelectedIndex=${ctrl.setSelectedIndex}
                                                />
                                                <${ProfileToggleField}
                                                    label="Push on local changes"
                                                    checked=${ctrl.form.push_on_change}
                                                    onChange=${(value) => ctrl.updateForm('push_on_change', value)}
                                                    surface=${ctrl.surfaceById.get('push-on-change')}
                                                    selectedIndex=${ctrl.selectedIndex}
                                                    setSelectedIndex=${ctrl.setSelectedIndex}
                                                />
                                            </div>
                                        </div>
                                    </div>
                                    ${ctrl.advancedProviderFields.length > 0 && html`
                                        <div class="profile-sync-grid">
                                            ${ctrl.advancedProviderFields.map(field => renderProviderField({
                                                field,
                                                form: ctrl.form,
                                                syncStatus: ctrl.syncStatus,
                                                branchOptions: ctrl.branchOptions,
                                                selectedIndex: ctrl.selectedIndex,
                                                setSelectedIndex: ctrl.setSelectedIndex,
                                                surface: ctrl.surfaceById.get(providerFieldSurfaceId(field.key)),
                                                updateForm: ctrl.updateForm,
                                            }))}
                                        </div>
                                    `}
                                </div>
                                <div class="profile-actions-footer">
                                    <${ProfileActionButton} surface=${ctrl.surfaceById.get('disconnect')} selectedIndex=${ctrl.selectedIndex} setSelectedIndex=${ctrl.setSelectedIndex} />
                                    <${ProfileActionButton} surface=${ctrl.surfaceById.get('pull')} selectedIndex=${ctrl.selectedIndex} setSelectedIndex=${ctrl.setSelectedIndex} />
                                    <${ProfileActionButton} surface=${ctrl.surfaceById.get('push')} selectedIndex=${ctrl.selectedIndex} setSelectedIndex=${ctrl.setSelectedIndex} />
                                    <${ProfileActionButton} surface=${ctrl.surfaceById.get('acknowledge')} selectedIndex=${ctrl.selectedIndex} setSelectedIndex=${ctrl.setSelectedIndex} />
                                    <${ProfileActionButton} surface=${ctrl.surfaceById.get('connect')} selectedIndex=${ctrl.selectedIndex} setSelectedIndex=${ctrl.setSelectedIndex} />
                                </div>
                            </section>

                            <section class="profile-section">
                                <div class="section-header">
                                    <h2>Backups</h2>
                                    <div class="section-actions">
                                        <span class="badge profile-backup-count">${ctrl.backups.length} backup${ctrl.backups.length === 1 ? '' : 's'}</span>
                                        ${ctrl.incident?.backup_file && html`
                                            <span class="badge profile-badge profile-badge-skipped">${ctrl.incident.backup_file}</span>
                                        `}
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
        ${ctrl.backupPreview && html`<${BackupPreviewModal} preview=${ctrl.backupPreview} onClose=${() => ctrl.setBackupPreview(null)} />`}
    `;
}
