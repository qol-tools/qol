import { html } from '../lib/html.js';
import { useCallback, useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { PageHeader } from '../components/PageHeader.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../components/app/view-keyboard-context.js';
import { useGridNav } from '../hooks/useGridNav.js';
import { toast } from '../lib/toast.js';
import { dispatchKey, withShiftVariants } from '../utils/keys.js';
import {
    acknowledgeProfileSync,
    connectProfileSync,
    disconnectProfileSync,
    exportProfile,
    importCounts,
    importSummary,
    openProfileBackupsDir,
    promptImportProfile,
    pullProfileSync,
    pushProfileSync,
} from './profile/actions.js';

const DEFAULT_BRANCH = 'main';
const DEFAULT_PATH = 'qol-tray/profile.json';
const DEFAULT_COMMIT_MESSAGE = 'chore: sync qol-tray profile';

function ProfileActionButton({ surface, selectedIndex, setSelectedIndex }) {
    if (!surface) {
        return null;
    }
    const selected = surface.index === selectedIndex;
    const classes = ['btn', surface.variant || 'btn-ghost', 'profile-action-btn', 'profile-nav-surface'];
    if (selected) {
        classes.push('is-selected');
    }
    return html`
        <button
            type="button"
            class=${classes.join(' ')}
            data-selected-surface=""
            data-selected=${selected ? 'true' : 'false'}
            data-index=${String(surface.index)}
            onFocus=${() => setSelectedIndex(surface.index)}
            onClick=${() => {
                setSelectedIndex(surface.index);
                surface.run?.();
            }}>
            ${surface.label}
        </button>
    `;
}

function ProfileInputField({
    id,
    label,
    hint = '',
    value,
    placeholder,
    type = 'text',
    surface,
    selectedIndex,
    setSelectedIndex,
    onInput,
}) {
    if (!surface) {
        return null;
    }
    const selected = surface.index === selectedIndex;
    return html`
        <div
            tabIndex="-1"
            class="form-group profile-input-surface"
            data-selected-surface=""
            data-selected=${selected ? 'true' : 'false'}
            data-index=${String(surface.index)}
            onMouseDown=${() => setSelectedIndex(surface.index)}
            onFocus=${() => setSelectedIndex(surface.index)}>
            <label for=${id}>
                ${label}
                ${hint && html`<span class="hint"> ${hint}</span>`}
            </label>
            <input
                id=${id}
                type=${type}
                class="profile-field-input"
                value=${value}
                placeholder=${placeholder}
                data-profile-editable=""
                onInput=${onInput}
            />
        </div>
    `;
}

function ProfileToggleField({
    label,
    checked,
    onChange,
    surface,
    selectedIndex,
    setSelectedIndex,
}) {
    if (!surface) {
        return null;
    }
    const selected = surface.index === selectedIndex;
    const toggle = () => onChange(!checked);
    return html`
        <div
            tabIndex="-1"
            class="toggle-inline profile-toggle-inline profile-toggle-surface"
            data-selected-surface=""
            data-selected=${selected ? 'true' : 'false'}
            data-index=${String(surface.index)}
            onMouseDown=${() => setSelectedIndex(surface.index)}
            onFocus=${() => setSelectedIndex(surface.index)}
            onClick=${toggle}>
            <div
                class=${`toggle-track ${checked ? 'on' : ''} profile-toggle-track`}
                tabIndex="-1"
                role="switch"
                aria-checked=${checked}>
                <div class="toggle-thumb"></div>
            </div>
            <span class="toggle-inline-label">${label}</span>
        </div>
    `;
}

function ImportFeedback({ lastImport }) {
    if (!lastImport) {
        return html`
            <p class="profile-empty">
                Import results appear here after you apply a profile bundle.
            </p>
        `;
    }

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

export function ProfileView({ syncStatus, onSyncStatusChange, refreshSyncStatus }) {
    const [busyAction, setBusyAction] = useState('');
    const [lastImport, setLastImport] = useState(null);
    const [selectedIndex, setSelectedIndex] = useState(0);
    const [formDirty, setFormDirty] = useState(false);
    const [form, setForm] = useState(() => createSyncForm(syncStatus));
    const selectedIndexRef = useRef(selectedIndex);
    selectedIndexRef.current = selectedIndex;

    const configured = Boolean(syncStatus?.configured);
    const incident = syncStatus?.incident || null;
    const syncBusy = busyAction === 'connect' || busyAction === 'pull' || busyAction === 'push' || busyAction === 'disconnect' || busyAction === 'acknowledge';
    const importBusy = busyAction === 'export' || busyAction === 'import';
    const syncStatusSeed = [
        syncStatus?.configured ? '1' : '0',
        syncStatus?.repo_url || '',
        syncStatus?.branch || '',
        syncStatus?.path || '',
        syncStatus?.commit_message || '',
        syncStatus?.pull_on_launch ? '1' : '0',
        syncStatus?.push_on_change ? '1' : '0',
    ].join('|');

    useEffect(() => {
        if (formDirty) {
            return;
        }
        setForm(createSyncForm(syncStatus));
    }, [syncStatusSeed, formDirty, syncStatus]);

    const applySyncStatus = useCallback((status) => {
        onSyncStatusChange?.(status);
        setForm(createSyncForm(status));
        setFormDirty(false);
    }, [onSyncStatusChange]);

    const updateForm = useCallback((key, value) => {
        setForm(current => ({ ...current, [key]: value }));
        setFormDirty(true);
    }, []);

    const handleExport = useCallback(async () => {
        setBusyAction('export');
        try {
            await exportProfile();
        } catch (error) {
            toast('error', `Failed to export profile: ${error.message}`);
        }
        setBusyAction('');
    }, []);

    const handleImport = useCallback(() => {
        promptImportProfile({
            onSelected: () => setBusyAction('import'),
            onImported: (result, file) => {
                setBusyAction('');
                setLastImport({
                    fileName: file?.name || 'qol-tray-profile.json',
                    result,
                });
                refreshSyncStatus?.();
            },
            onError: (error) => {
                setBusyAction('');
                toast('error', `Failed to import profile: ${error.message}`);
            },
        });
    }, [refreshSyncStatus]);

    const handleReload = useCallback(() => {
        window.location.reload();
    }, []);

    const handleConnect = useCallback(async () => {
        setBusyAction('connect');
        try {
            const result = await connectProfileSync(form);
            applySyncStatus(result.status);
        } catch (error) {
            toast('error', `Failed to save cloud sync: ${error.message}`);
        }
        setBusyAction('');
    }, [applySyncStatus, form]);

    const handlePull = useCallback(async () => {
        setBusyAction('pull');
        try {
            const result = await pullProfileSync();
            applySyncStatus(result.status);
        } catch (error) {
            toast('error', `Failed to pull cloud sync: ${error.message}`);
        }
        setBusyAction('');
    }, [applySyncStatus]);

    const handlePush = useCallback(async () => {
        setBusyAction('push');
        try {
            const result = await pushProfileSync();
            applySyncStatus(result.status);
        } catch (error) {
            toast('error', `Failed to push cloud sync: ${error.message}`);
        }
        setBusyAction('');
    }, [applySyncStatus]);

    const handleDisconnect = useCallback(async () => {
        setBusyAction('disconnect');
        try {
            const result = await disconnectProfileSync();
            applySyncStatus(result.status);
        } catch (error) {
            toast('error', `Failed to disconnect cloud sync: ${error.message}`);
        }
        setBusyAction('');
    }, [applySyncStatus]);

    const handleAcknowledge = useCallback(async () => {
        setBusyAction('acknowledge');
        try {
            const result = await acknowledgeProfileSync();
            applySyncStatus(result.status);
        } catch (error) {
            toast('error', `Failed to acknowledge cloud sync review: ${error.message}`);
        }
        setBusyAction('');
    }, [applySyncStatus]);

    const handleOpenBackups = useCallback(async () => {
        try {
            await openProfileBackupsDir();
        } catch (error) {
            toast('error', `Failed to open backups folder: ${error.message}`);
        }
    }, []);

    const surfaces = useMemo(() => {
        const next = [];
        const add = (id, kind, extra = {}) => {
            next.push({ id, kind, index: next.length, ...extra });
        };

        add('repo-url', 'field');
        add('token', 'field');
        add('branch', 'field');
        add('path', 'field');
        add('commit-message', 'field');
        add('pull-on-launch', 'toggle', {
            run: () => updateForm('pull_on_launch', !form.pull_on_launch),
        });
        add('push-on-change', 'toggle', {
            run: () => updateForm('push_on_change', !form.push_on_change),
        });
        add('connect', 'action', {
            label: configured ? 'Save and Sync' : 'Connect GitHub Sync',
            variant: 'btn-primary',
            run: syncBusy ? null : handleConnect,
        });
        if (configured) {
            add('pull', 'action', { label: 'Pull Now', run: syncBusy ? null : handlePull });
            add('push', 'action', { label: 'Push Now', run: syncBusy ? null : handlePush });
            add('disconnect', 'action', { label: 'Disconnect', run: syncBusy ? null : handleDisconnect });
        }
        if (incident) {
            add('acknowledge', 'action', {
                label: 'Acknowledge',
                variant: 'btn-primary',
                run: syncBusy ? null : handleAcknowledge,
            });
        }
        add('export', 'action', {
            label: 'Export Profile',
            variant: 'btn-primary',
            run: importBusy ? null : handleExport,
        });
        add('import', 'action', {
            label: 'Import Profile',
            variant: 'btn-primary',
            run: importBusy ? null : handleImport,
        });
        add('reload', 'action', { label: 'Reload Dashboard', run: handleReload });
        add('open-backups', 'action', { label: 'Open Backups Folder', run: handleOpenBackups });

        return next;
    }, [
        configured,
        form,
        handleAcknowledge,
        handleConnect,
        handleDisconnect,
        handleExport,
        handleImport,
        handleOpenBackups,
        handlePull,
        handlePush,
        handleReload,
        importBusy,
        incident,
        syncBusy,
        updateForm,
    ]);

    useEffect(() => {
        setSelectedIndex(index => Math.min(index, Math.max(0, surfaces.length - 1)));
    }, [surfaces.length]);

    const surfaceById = useMemo(
        () => new Map(surfaces.map(surface => [surface.id, surface])),
        [surfaces]
    );
    const navigateInGrid = useGridNav('#profile-sections [data-selected-surface]', selectedIndexRef, setSelectedIndex);
    const activateSelected = useCallback(() => {
        const surface = surfaces[selectedIndex];
        if (!surface) {
            return;
        }
        if (surface.kind === 'action') {
            surface.run?.();
            return;
        }
        if (surface.kind === 'toggle') {
            surface.run?.();
            return;
        }
        enterSurfaceEditor(surface.index);
    }, [selectedIndex, surfaces]);

    const handleTextInputNavigation = useCallback((event, active) => {
        if (event.key === 'Escape' || event.key === 'Enter') {
            event.preventDefault();
            focusSurfaceContainer(selectedIndexRef.current);
            return true;
        }
        const direction = arrowDirection(event.key);
        if (!direction) {
            return false;
        }
        if ((direction === 'left' || direction === 'right') && shouldKeepHorizontalCaret(event, active)) {
            return false;
        }
        event.preventDefault();
        navigateInGrid(direction);
        return true;
    }, [navigateInGrid]);

    const handleKey = useCallback((event) => {
        const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        if (isTextSurface(active)) {
            if (handleTextInputNavigation(event, active)) {
                return;
            }
            return;
        }
        dispatchKey(event, withShiftVariants({
            ArrowLeft: () => navigateInGrid('left'),
            ArrowRight: () => navigateInGrid('right'),
            ArrowUp: () => navigateInGrid('up'),
            ArrowDown: () => navigateInGrid('down'),
            h: () => navigateInGrid('left'),
            l: () => navigateInGrid('right'),
            k: () => navigateInGrid('up'),
            j: () => navigateInGrid('down'),
            Enter: activateSelected,
            ' ': activateSelected,
        }));
    }, [activateSelected, handleTextInputNavigation, navigateInGrid]);
    const isBlocking = useCallback(() => isTextSurface(document.activeElement), []);
    useRegisterViewKeyboard('profile', handleKey, isBlocking);

    const commands = useMemo(() => {
        const next = [
            { id: 'profile:export', label: 'Export profile', run: handleExport },
            { id: 'profile:import', label: 'Import profile', run: handleImport },
            { id: 'profile:reload', label: 'Reload dashboard', run: handleReload },
            { id: 'profile:backups', label: 'Open backups folder', run: handleOpenBackups },
        ];
        if (configured) {
            next.unshift(
                { id: 'profile:push', label: 'Push cloud sync', run: handlePush },
                { id: 'profile:pull', label: 'Pull cloud sync', run: handlePull },
            );
        }
        if (incident) {
            next.unshift({ id: 'profile:acknowledge', label: 'Acknowledge sync review', run: handleAcknowledge });
        }
        next.unshift({
            id: 'profile:connect',
            label: configured ? 'Save cloud sync' : 'Connect GitHub sync',
            run: handleConnect,
        });
        return next;
    }, [
        configured,
        handleAcknowledge,
        handleConnect,
        handleExport,
        handleImport,
        handleOpenBackups,
        handlePull,
        handlePush,
        handleReload,
        incident,
    ]);
    useRegisterCommands('profile', commands);

    return html`
        <div class="view-container content-shell profile-view-shell">
            <${PageHeader}
                title="Profile"
                subtitle="Cloud sync, import and export, and recovery for your QoL Tray setup"
                badge=${html`<${ProfileHeaderBadge} syncStatus=${syncStatus} />`}
            />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <div id="profile-sections" class="content-frame profile-frame">
                        <section class="profile-section">
                            <div class="section-header">
                                <h2>Cloud Sync</h2>
                            </div>
                            <p class="section-desc">
                                GitHub is the first cloud provider. It syncs the canonical
                                profile model and reuses the same credential for the plugin store.
                            </p>
                            <div class="profile-sync-summary" data-health=${syncStatus?.health || 'not_configured'}>
                                <div class="profile-sync-summary-head">
                                    <div class="profile-sync-summary-title">
                                        <span class="profile-health-dot" data-health=${syncStatus?.health || 'not_configured'}></span>
                                        <span>${profileHealthLabel(syncStatus)}</span>
                                    </div>
                                    ${syncStatus?.last_sync_at && html`
                                        <span class="profile-sync-summary-meta">
                                            Last sync ${formatTimestamp(syncStatus.last_sync_at)}
                                        </span>
                                    `}
                                </div>
                                ${syncStatus?.repo_url && html`
                                    <div class="profile-sync-summary-meta">
                                        ${syncStatus.provider_label || 'GitHub'} · ${syncStatus.repo_url}
                                    </div>
                                `}
                                ${syncStatus?.incident && html`
                                    <p class="profile-sync-message profile-sync-message-warning">
                                        ${syncStatus.incident.message}
                                    </p>
                                `}
                                ${syncStatus?.last_error && html`
                                    <p class="profile-sync-message profile-sync-message-error">
                                        ${syncStatus.last_error}
                                    </p>
                                `}
                            </div>
                            <div class="profile-form-grid">
                                <${ProfileInputField}
                                    id="profile-repo-url"
                                    label="GitHub repo URL"
                                    value=${form.repo_url}
                                    placeholder="https://github.com/owner/repo.git"
                                    surface=${surfaceById.get('repo-url')}
                                    selectedIndex=${selectedIndex}
                                    setSelectedIndex=${setSelectedIndex}
                                    onInput=${(event) => updateForm('repo_url', event.currentTarget.value)}
                                />
                                <${ProfileInputField}
                                    id="profile-token"
                                    type="password"
                                    label="GitHub PAT"
                                    hint="leave blank to keep the stored PAT"
                                    value=${form.token}
                                    placeholder=${syncStatus?.has_github_token ? 'Stored PAT on file' : 'Paste PAT'}
                                    surface=${surfaceById.get('token')}
                                    selectedIndex=${selectedIndex}
                                    setSelectedIndex=${setSelectedIndex}
                                    onInput=${(event) => updateForm('token', event.currentTarget.value)}
                                />
                                <${ProfileInputField}
                                    id="profile-branch"
                                    label="Branch"
                                    value=${form.branch}
                                    placeholder=${DEFAULT_BRANCH}
                                    surface=${surfaceById.get('branch')}
                                    selectedIndex=${selectedIndex}
                                    setSelectedIndex=${setSelectedIndex}
                                    onInput=${(event) => updateForm('branch', event.currentTarget.value)}
                                />
                                <${ProfileInputField}
                                    id="profile-path"
                                    label="Remote path"
                                    value=${form.path}
                                    placeholder=${DEFAULT_PATH}
                                    surface=${surfaceById.get('path')}
                                    selectedIndex=${selectedIndex}
                                    setSelectedIndex=${setSelectedIndex}
                                    onInput=${(event) => updateForm('path', event.currentTarget.value)}
                                />
                                <div class="form-group profile-form-span-2">
                                    <label for="profile-commit-message">Commit message</label>
                                    <div
                                        tabIndex="-1"
                                        class="profile-input-surface"
                                        data-selected-surface=""
                                        data-selected=${surfaceById.get('commit-message')?.index === selectedIndex ? 'true' : 'false'}
                                        data-index=${String(surfaceById.get('commit-message')?.index ?? -1)}
                                        onMouseDown=${() => setSelectedIndex(surfaceById.get('commit-message')?.index ?? 0)}
                                        onFocus=${() => setSelectedIndex(surfaceById.get('commit-message')?.index ?? 0)}>
                                        <input
                                            id="profile-commit-message"
                                            type="text"
                                            class="profile-field-input"
                                            value=${form.commit_message}
                                            placeholder=${DEFAULT_COMMIT_MESSAGE}
                                            data-profile-editable=""
                                            onInput=${(event) => updateForm('commit_message', event.currentTarget.value)}
                                        />
                                    </div>
                                </div>
                            </div>
                            <div class="profile-toggle-row">
                                <${ProfileToggleField}
                                    label="Pull on launch"
                                    checked=${form.pull_on_launch}
                                    onChange=${(value) => updateForm('pull_on_launch', value)}
                                    surface=${surfaceById.get('pull-on-launch')}
                                    selectedIndex=${selectedIndex}
                                    setSelectedIndex=${setSelectedIndex}
                                />
                                <${ProfileToggleField}
                                    label="Push on local changes"
                                    checked=${form.push_on_change}
                                    onChange=${(value) => updateForm('push_on_change', value)}
                                    surface=${surfaceById.get('push-on-change')}
                                    selectedIndex=${selectedIndex}
                                    setSelectedIndex=${setSelectedIndex}
                                />
                            </div>
                            <div class="profile-actions">
                                <${ProfileActionButton} surface=${surfaceById.get('connect')} selectedIndex=${selectedIndex} setSelectedIndex=${setSelectedIndex} />
                                <${ProfileActionButton} surface=${surfaceById.get('pull')} selectedIndex=${selectedIndex} setSelectedIndex=${setSelectedIndex} />
                                <${ProfileActionButton} surface=${surfaceById.get('push')} selectedIndex=${selectedIndex} setSelectedIndex=${setSelectedIndex} />
                                <${ProfileActionButton} surface=${surfaceById.get('disconnect')} selectedIndex=${selectedIndex} setSelectedIndex=${setSelectedIndex} />
                                <${ProfileActionButton} surface=${surfaceById.get('acknowledge')} selectedIndex=${selectedIndex} setSelectedIndex=${setSelectedIndex} />
                            </div>
                        </section>

                        <section class="profile-section">
                            <div class="section-header">
                                <h2>Import / Export</h2>
                            </div>
                            <p class="section-desc">
                                Manual profile transfer uses the same canonical payload as cloud sync.
                            </p>
                            <div class="profile-actions">
                                <${ProfileActionButton} surface=${surfaceById.get('export')} selectedIndex=${selectedIndex} setSelectedIndex=${setSelectedIndex} />
                                <${ProfileActionButton} surface=${surfaceById.get('import')} selectedIndex=${selectedIndex} setSelectedIndex=${setSelectedIndex} />
                                <${ProfileActionButton} surface=${surfaceById.get('reload')} selectedIndex=${selectedIndex} setSelectedIndex=${setSelectedIndex} />
                            </div>
                            <${ImportFeedback} lastImport=${lastImport} />
                        </section>

                        <section class="profile-section">
                            <div class="section-header">
                                <h2>Backups</h2>
                            </div>
                            <p class="section-desc">
                                Local profile backups are created before recovery flows apply a remote state over local changes.
                            </p>
                            <div class="profile-backup-grid">
                                <div class="profile-backup-stat">
                                    <span class="profile-backup-label">Backups</span>
                                    <strong>${String(syncStatus?.backup_count || 0)}</strong>
                                </div>
                                <div class="profile-backup-stat profile-backup-stat-wide">
                                    <span class="profile-backup-label">Latest backup</span>
                                    <strong>${syncStatus?.latest_backup_file || 'No backups yet'}</strong>
                                </div>
                                <div class="profile-backup-stat profile-backup-stat-wide">
                                    <span class="profile-backup-label">Folder</span>
                                    <strong>${syncStatus?.backups_dir || 'Unavailable'}</strong>
                                </div>
                                ${incident?.backup_file && html`
                                    <div class="profile-backup-stat profile-backup-stat-wide">
                                        <span class="profile-backup-label">Review backup</span>
                                        <strong>${incident.backup_file}</strong>
                                    </div>
                                `}
                            </div>
                            <div class="profile-actions">
                                <${ProfileActionButton} surface=${surfaceById.get('open-backups')} selectedIndex=${selectedIndex} setSelectedIndex=${setSelectedIndex} />
                            </div>
                        </section>
                    </div>
                </div>
            </div>
        </div>
    `;
}

function ProfileHeaderBadge({ syncStatus }) {
    const health = syncStatus?.health || 'not_configured';
    return html`
        <div class="profile-header-badge" data-health=${health}>
            <span class="profile-health-dot" data-health=${health}></span>
            <span>${profileHealthLabel(syncStatus)}</span>
        </div>
    `;
}

function profileHealthLabel(syncStatus) {
    const health = syncStatus?.health || 'not_configured';
    if (health === 'healthy') {
        return 'Synced and healthy';
    }
    if (health === 'attention') {
        return 'Review required';
    }
    if (health === 'error') {
        return 'Sync error';
    }
    return 'Cloud sync not configured';
}

function createSyncForm(syncStatus) {
    return {
        token: '',
        repo_url: syncStatus?.repo_url || '',
        branch: syncStatus?.branch || DEFAULT_BRANCH,
        path: syncStatus?.path || DEFAULT_PATH,
        commit_message: syncStatus?.commit_message || DEFAULT_COMMIT_MESSAGE,
        pull_on_launch: syncStatus?.pull_on_launch ?? true,
        push_on_change: syncStatus?.push_on_change ?? true,
    };
}

function buildBadges(counts) {
    const badges = [];
    if (counts.installed) badges.push({ label: `${counts.installed} installed`, className: 'profile-badge-install' });
    if (counts.updated) badges.push({ label: `${counts.updated} updated`, className: 'profile-badge-update' });
    if (counts.kept) badges.push({ label: `${counts.kept} unchanged`, className: 'profile-badge-kept' });
    if (counts.skipped) badges.push({ label: `${counts.skipped} skipped`, className: 'profile-badge-skipped' });
    if (counts.failed) badges.push({ label: `${counts.failed} failed`, className: 'profile-badge-failed' });
    if (badges.length > 0) return badges;
    return [{ label: 'No plugin actions', className: 'profile-badge-kept' }];
}

function formatTimestamp(value) {
    if (!value) {
        return '';
    }
    try {
        return new Date(value).toLocaleString();
    } catch {
        return value;
    }
}

function isTextSurface(element) {
    return element?.matches?.('[data-profile-editable], textarea, [contenteditable="true"]');
}

function arrowDirection(key) {
    if (key === 'ArrowUp') return 'up';
    if (key === 'ArrowDown') return 'down';
    if (key === 'ArrowLeft') return 'left';
    if (key === 'ArrowRight') return 'right';
    return null;
}

function shouldKeepHorizontalCaret(event, active) {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') {
        return false;
    }
    if (!(active instanceof HTMLInputElement) && !(active instanceof HTMLTextAreaElement)) {
        return false;
    }
    if (active.readOnly || active.disabled) {
        return false;
    }
    if (active.selectionStart === null || active.selectionEnd === null) {
        return false;
    }
    if (active.selectionStart !== active.selectionEnd) {
        return true;
    }
    if (event.key === 'ArrowLeft') {
        return active.selectionStart > 0;
    }
    return active.selectionEnd < active.value.length;
}

function enterSurfaceEditor(index) {
    const container = surfaceElement(index);
    if (!(container instanceof HTMLElement)) {
        return;
    }
    const input = container.querySelector('[data-profile-editable]');
    if (input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement) {
        input.focus();
        input.select?.();
    }
}

function focusSurfaceContainer(index) {
    const element = surfaceElement(index);
    if (!(element instanceof HTMLElement)) {
        return;
    }
    element.focus();
}

function surfaceElement(index) {
    return document.querySelector(`#profile-sections [data-selected-surface][data-index="${index}"]`);
}
