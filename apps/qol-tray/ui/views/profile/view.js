import { html } from '../../lib/html.js';
import { useEffect, useLayoutEffect, useRef, useState } from 'preact/hooks';
import { PageHeader } from '../../components/PageHeader.js';
import { SurfaceContainer } from '../../lib/components/SurfaceContainer.js';
import { Surface, useInputSurface } from '../../lib/components/Surface.js';
import { Badge, HealthDot, Alert } from '../../lib/components/StatusIndicators.js';
import { AuthHealthBanner } from '../../lib/components/AuthHealthBanner.js';
import { insufficientScopeIssue } from '../../features/auth/actions.js';
import { BackupDetailContent } from '../../components/domain-rows/BackupRow.js';
import { IconChevron } from '../../assets/icon-chevron.js';
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
import { importProfileText, openProfileBackupFile } from './actions.js';
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
    const settingsPanelRef = useRef(null);
    const shouldDiveSettingsRef = useRef(false);

    useEffect(() => {
        backupPreviewSlot.set({
            preview: ctrl.backupPreview,
            incident: ctrl.incident,
            onAcknowledge: ctrl.handleAcknowledge,
        });
    }, [ctrl.backupPreview, ctrl.incident, ctrl.handleAcknowledge]);

    const health = ctrl.syncStatus?.health || 'not_configured';
    const settingsSurface = ctrl.surfaceById.get('settings');
    const githubAuthIssue = insufficientScopeIssue(ctrl.authHealth, 'github');
    const reauthBusy = ctrl.busyAction === 'reauth';
    const toggleSettings = () => {
        ctrl.setSelectedIndex(settingsSurface?.index);
        setShowSettings(current => {
            const next = !current;
            shouldDiveSettingsRef.current = next;
            return next;
        });
    };
    const settingsTrigger = useInputSurface({
        index: settingsSurface?.index,
        selected: settingsSurface?.index === ctrl.selectedIndex,
        onSelect: ctrl.setSelectedIndex,
        onActivate: toggleSettings,
    });
    const closeSettingsPanel = (event) => {
        if (event.key !== 'Escape') return;
        event.preventDefault();
        event.stopPropagation();
        shouldDiveSettingsRef.current = false;
        settingsTrigger.ref.current?.removeAttribute('data-dive-source');
        settingsTrigger.ref.current?.focus?.({ preventScroll: true });
        ctrl.setSelectedIndex(settingsSurface?.index);
        setShowSettings(false);
    };

    useLayoutEffect(() => {
        if (!showSettings) {
            settingsTrigger.ref.current?.removeAttribute('data-dive-source');
            return;
        }
        if (!shouldDiveSettingsRef.current) return;
        shouldDiveSettingsRef.current = false;
        settingsTrigger.ref.current?.setAttribute('data-dive-source', '');
        const first = settingsPanelRef.current?.querySelector('[data-selected-surface]');
        first?.focus?.({ preventScroll: true });
    }, [showSettings]);

    return html`
        <div id="profile-page" class="view-container content-shell profile-view-shell">
            <${PageHeader} subtitle="Cloud sync, import and export, and recovery for your QoL Tray setup" />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <div class="profile-page-stack">
                        <${SurfaceContainer} id="profile-sections" className="content-frame profile-frame">
                            ${ctrl.lastImport && html`<${ImportFeedback} lastImport=${ctrl.lastImport} />`}

                            <section class="profile-section">
                                <div class="profile-status-line">
                                    <${HealthDot} health=${health} />
                                    <span class="profile-status-label">${profileHealthLabel(ctrl.syncStatus)}</span>
                                    ${ctrl.configured && html`
                                        <span class="profile-status-meta">${'\u00b7'} ${profileLastSyncSummary(ctrl.syncStatus)}</span>
                                    `}
                                </div>

                                ${ctrl.syncStatus?.incident && html`
                                    <${Alert} variant="warning">${ctrl.syncStatus.incident.message}<//>
                                `}
                                ${ctrl.syncStatus?.last_error && html`
                                    <${Alert} variant="error">${ctrl.syncStatus.last_error}<//>
                                `}

                                ${!ctrl.authPrompt && githubAuthIssue && html`
                                    <${AuthHealthBanner}
                                        issue=${githubAuthIssue}
                                        busy=${reauthBusy}
                                        onReauthorize=${ctrl.handleReauthorize}
                                    />
                                `}

                                ${ctrl.authPrompt && html`
                                    <${DeviceCodePrompt}
                                        userCode=${ctrl.authPrompt.userCode}
                                        verificationUri=${ctrl.authPrompt.verificationUri}
                                        copied=${ctrl.authPrompt.copied}
                                        onOpenGitHub=${ctrl.openAuthLink}
                                    />
                                `}

                                ${!ctrl.authPrompt && ctrl.configured && html`
                                    <div class="profile-actions-row">
                                        ${!githubAuthIssue && html`
                                            <${ProfileActionButton} id="pull" ctrl=${ctrl} />
                                            <${ProfileActionButton} id="push" ctrl=${ctrl} />
                                        `}
                                        <${ProfileActionButton} id="acknowledge" ctrl=${ctrl} />
                                        <span class="profile-actions-spacer"></span>
                                        <${ProfileActionButton} id="disconnect" ctrl=${ctrl} />
                                    </div>
                                `}

                                ${!ctrl.authPrompt && !githubAuthIssue && !ctrl.configured && html`
                                    <div class="profile-connect-row">
                                        <${ProfileActionButton} id="connect" ctrl=${ctrl} />
                                    </div>
                                `}

                                ${!ctrl.authPrompt && html`
                                    <div class="profile-settings-group" data-expanded=${showSettings ? 'true' : 'false'}>
                                        <button
                                            ref=${settingsTrigger.ref}
                                            class=${`btn btn-ghost profile-settings-trigger ${showSettings ? 'is-open' : ''}`}
                                            type="button"
                                            aria-expanded=${showSettings ? 'true' : 'false'}
                                            ...${settingsTrigger.attrs}
                                        >
                                            <span class="btn-icon btn-icon-chevron"><${IconChevron} size=${11} /></span>
                                            <span>Settings</span>
                                        </button>
                                        ${showSettings && html`
                                        <${SurfaceContainer} className="profile-settings-panel" containerRef=${settingsPanelRef} onKeyDown=${closeSettingsPanel}>
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
                                        `}
                                    </div>
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

export const prodBackupDetailConfig = {
    formatText: formatBackupPreview,
    onClose: dispatchEscape,
    onOpenExternal: (fileName) => openProfileBackupFile(fileName)
        .catch((err) => toast('error', `Failed to open: ${err.message}`)),
    onCopy: (content) => {
        navigator.clipboard.writeText(content);
        toast('success', 'Copied to clipboard');
    },
    onRestore: (content) => importProfileText(content)
        .then(() => dispatchEscape())
        .catch((err) => toast('error', `Failed to restore backup: ${err.message}`)),
    onAcknowledge: (slotAcknowledge) => { slotAcknowledge?.(); dispatchEscape(); },
};

export function BackupDetailSubPage({ slot, config }) {
    const [, bump] = useState(0);
    useEffect(() => slot.subscribe(() => bump(t => t + 1)), [slot]);

    const { preview, incident, onAcknowledge } = slot.get();
    if (!preview) {
        return html`<div class="view-container content-shell">
            <${PageHeader} title="Backup Preview" subtitle="Select a backup to view" />
        </div>`;
    }
    const isIncidentBackup = incident?.backup_file === preview.file_name;
    return html`
        <div class="view-container content-shell">
            <${PageHeader} title=${preview.file_name} subtitle=${isIncidentBackup ? 'Backup awaiting review' : 'Backup preview'} />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame backup-detail-frame">
                        <${BackupDetailContent}
                            text=${config.formatText(preview.content)}
                            isIncidentBackup=${isIncidentBackup}
                            onClose=${config.onClose}
                            onOpenExternal=${() => config.onOpenExternal(preview.file_name)}
                            onCopy=${() => config.onCopy(preview.content)}
                            onRestore=${() => config.onRestore(preview.content)}
                            onAcknowledge=${() => config.onAcknowledge(onAcknowledge)} />
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

function DeviceCodePrompt({ userCode, verificationUri, copied, onOpenGitHub }) {
    const [justCopiedCode, setJustCopiedCode] = useState(copied);
    const [justCopiedUri, setJustCopiedUri] = useState(false);
    const containerRef = useRef(null);

    useEffect(() => {
        const selected = containerRef.current?.querySelector('[data-selected="true"]');
        const target = selected || containerRef.current?.querySelector('[data-selected-surface]');
        target?.focus?.({ preventScroll: true });
    }, []);

    const copyToClipboard = async (text, setFlag) => {
        if (!text) return;
        try {
            await navigator.clipboard.writeText(text);
            setFlag(true);
            setTimeout(() => setFlag(false), 2000);
        } catch (_) {}
    };

    const copyCode = () => copyToClipboard(userCode, setJustCopiedCode);
    const copyUri = () => copyToClipboard(verificationUri, setJustCopiedUri);

    return html`
        <div class="profile-device-auth" ref=${containerRef}>
            <${Surface} as="button"
                className="profile-device-code btn"
                onActivate=${copyCode}
                aria-label="Copy verification code ${userCode}"
                title="Copy code"
            >${userCode}<//>
            <p class="profile-device-hint">
                ${justCopiedCode ? 'Code copied' : 'Enter on code to copy'} - then authorize on GitHub
            </p>
            ${verificationUri && html`
                <${Surface} as="div"
                    className="profile-device-uri"
                    onActivate=${copyUri}
                    aria-label="Copy verification URL ${verificationUri}"
                    title="Copy URL"
                >${verificationUri}<//>
                <p class="profile-device-hint profile-device-hint-secondary">
                    ${justCopiedUri ? 'URL copied' : 'Enter on URL to copy - paste into a non-incognito browser if Open GitHub lands in the wrong window'}
                </p>
            `}
            <${Surface} as="button"
                className="btn btn-primary"
                selected=${true}
                onActivate=${onOpenGitHub}
            >Open GitHub<//>
            <p class="profile-device-status">
                <span class="profile-action-spinner" aria-hidden="true"></span>
                Waiting for GitHub authorization...
            </p>
        </div>
    `;
}
