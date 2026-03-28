import { html } from '../lib/html.js';
import { useCallback, useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { PageHeader } from '../components/PageHeader.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../components/app/view-keyboard-context.js';
import { useGridNav } from '../hooks/useGridNav.js';
import { toast } from '../lib/toast.js';
import { dispatchKey, withShiftVariants } from '../utils/keys.js';
import {
    exportProfile,
    importCounts,
    importSummary,
    promptImportProfile,
} from './profile/actions.js';

function ProfileActionCard({ action, index, selected, onSelect, onActivate }) {
    const className = `profile-card profile-action-card${selected ? ' is-selected' : ''}${action.disabled ? ' is-disabled' : ''}`;

    return html`
        <section
            class=${className}
            data-selected-surface=""
            data-selected=${selected ? 'true' : 'false'}
            data-index=${String(index)}
            onClick=${() => selected ? onActivate(index) : onSelect(index)}>
            <div class="profile-card-head">
                <h3 data-selected-text="">${action.title}</h3>
                <p data-selected-text="">${action.description}</p>
            </div>
            ${action.items?.length > 0 && html`
                <ul class="profile-list">
                    ${action.items.map(item => html`<li key=${item}>${item}</li>`)}
                </ul>
            `}
            <div class="profile-actions">
                <span class=${`btn ${action.variant || 'btn-primary'}`}>${action.label}</span>
            </div>
        </section>
    `;
}

function ImportFeedback({ lastImport, onReload }) {
    if (!lastImport) {
        return html`
            <p class="profile-empty">
                No import has run yet. Results and per-plugin restore status will appear here.
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
                <button class="btn btn-ghost btn-sm" onClick=${onReload}>Reload Dashboard</button>
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

export function ProfileView() {
    const [busyAction, setBusyAction] = useState('');
    const [lastImport, setLastImport] = useState(null);
    const [selectedIndex, setSelectedIndex] = useState(0);
    const selectedIndexRef = useRef(selectedIndex);
    selectedIndexRef.current = selectedIndex;

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
            },
            onError: (error) => {
                setBusyAction('');
                toast('error', `Failed to import profile: ${error.message}`);
            },
        });
    }, []);

    const handleReload = useCallback(() => {
        window.location.reload();
    }, []);

    const actions = useMemo(() => {
        const next = [
            {
                id: 'export',
                title: 'Export',
                label: busyAction === 'export' ? 'Exporting…' : 'Export Profile',
                description:
                    'Create a portable profile bundle with hotkeys, shortcuts, task runner actions, plugin config, and pinned plugin versions.',
                items: [
                    'Portable JSON file for another machine',
                    'Exact plugin restore metadata',
                    'Canonical profile state, not browser local storage',
                ],
                variant: 'btn-primary',
                disabled: busyAction.length > 0,
                run: handleExport,
            },
            {
                id: 'import',
                title: 'Import',
                label: busyAction === 'import' ? 'Importing…' : 'Import Profile',
                description:
                    'Restore a saved profile bundle and reconcile the plugin set against the pinned versions inside it.',
                items: [
                    'Applies core settings immediately',
                    'Reinstalls or updates plugins from the bundle',
                    'Keeps unsupported plugins as skipped instead of dropping them',
                ],
                variant: 'btn-primary',
                disabled: busyAction.length > 0,
                run: handleImport,
            },
        ];

        if (!lastImport) return next;

        return [
            ...next,
            {
                id: 'reload',
                title: 'Reload',
                label: 'Reload Dashboard',
                description:
                    'Refresh visible dashboard state after an import without restarting qol-tray.',
                items: [
                    'Useful after core settings or plugin reconcile changes',
                ],
                variant: 'btn-ghost',
                disabled: false,
                run: handleReload,
            },
        ];
    }, [busyAction, handleExport, handleImport, handleReload, lastImport]);

    useEffect(() => {
        setSelectedIndex(index => Math.min(index, Math.max(0, actions.length - 1)));
    }, [actions.length]);

    const activateSelected = useCallback(() => {
        const action = actions[selectedIndex];
        if (!action || action.disabled) return;
        action.run();
    }, [actions, selectedIndex]);

    const navigateInGrid = useGridNav('#profile-grid .profile-action-card', selectedIndexRef, setSelectedIndex);
    const handleKey = useCallback((event) => {
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
        }));
    }, [activateSelected, navigateInGrid]);
    useRegisterViewKeyboard('profile', handleKey);

    const commands = useMemo(
        () => [
            { id: 'profile:export', label: 'Export profile', run: handleExport },
            { id: 'profile:import', label: 'Import profile', run: handleImport },
            { id: 'profile:reload', label: 'Reload dashboard', run: handleReload },
        ],
        [handleExport, handleImport, handleReload]
    );
    useRegisterCommands('profile', commands);

    return html`
        <div class="view-container content-shell">
            <${PageHeader}
                title="Profile"
                subtitle="Portable import and restore for your full QoL Tray setup"
            />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <div class="content-frame profile-frame">
                        <div id="profile-grid" class="profile-grid">
                            ${actions.map((action, index) => html`
                                <${ProfileActionCard}
                                    key=${action.id}
                                    action=${action}
                                    index=${index}
                                    selected=${index === selectedIndex}
                                    onSelect=${setSelectedIndex}
                                    onActivate=${activateSelected}
                                />
                            `)}
                        </div>
                        <section class="profile-feedback-card">
                            <div class="profile-card-head">
                                <h3>Last Import</h3>
                                <p>
                                    This is the manual transport the future sync feature should
                                    build on top of.
                                </p>
                            </div>
                            <${ImportFeedback} lastImport=${lastImport} onReload=${handleReload} />
                        </section>
                    </div>
                </div>
            </div>
        </div>
    `;
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
