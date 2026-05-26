import { useCallback, useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { useRouter } from '../hooks/useRouter.js';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { GLOBAL_ID } from '../palette/registry.js';
import { useAppBootstrap } from './useAppBootstrap.js';
import { useAppUpdateCoordinator } from './useAppUpdateCoordinator.js';
import { useMountedViews } from './useMountedViews.js';
import { useSidebarActions } from './useSidebarActions.js';
import { buildViewOrder, getViewLabel } from './views.js';
import {
    exportProfile,
    fetchSyncProviders,
    fetchSyncStatus,
    promptImportProfile,
} from '../views/profile/actions.js';
import { toast } from '../lib/toast.js';
import { resolveInitialBranch } from './worktree-selection.js';

const BRANCH_KEY = 'dev.recompile.defaultBranch';
const LEGACY_PATH_KEY = 'dev.recompile.defaultWorktree';
const SYNC_STATUS_POLL_MS = 5000;

function readDefaultBranch() {
    try {
        const current = localStorage.getItem(BRANCH_KEY);
        if (current) return current;
        if (localStorage.getItem(LEGACY_PATH_KEY)) localStorage.removeItem(LEGACY_PATH_KEY);
        return null;
    } catch { return null; }
}

export function useApp({ onDissolve } = {}) {
    const { devEnabled, appVersion } = useAppBootstrap();
    const viewOrder = useMemo(() => buildViewOrder(devEnabled), [devEnabled]);
    const { activeViewId, activePluginId, switchView, openPluginConfig, closePluginConfig } = useRouter({ viewOrder });
    const mounted = useMountedViews(activeViewId);
    const { updateState, checkForUpdate, beginSelfUpdate, failSelfUpdate, beginDevRecompile, failDevRecompile } = useAppUpdateCoordinator({ devEnabled, appVersion, onDissolve });
    const [branches, setBranches] = useState([]);
    const [repoBranch, setRepoBranch] = useState(null);
    const [defaultBranch, setDefaultBranchState] = useState(readDefaultBranch);
    const [syncStatus, setSyncStatus] = useState(defaultSyncStatus);
    const [syncProviders, setSyncProviders] = useState([]);
    const defaultBranchRef = useRef(defaultBranch);
    defaultBranchRef.current = defaultBranch;
    const setDefaultBranch = useCallback(branch => {
        const v = branch || null;
        defaultBranchRef.current = v;
        try {
            if (v) localStorage.setItem(BRANCH_KEY, v);
            if (!v) localStorage.removeItem(BRANCH_KEY);
        } catch {}
        setDefaultBranchState(v);
    }, []);
    useEffect(() => {
        if (!devEnabled) return;
        let cancelled = false;
        Promise.all([
            fetch('/api/dev/worktrees').then(r => r.ok ? r.json() : []).catch(() => []),
            fetch('/api/dev/active-worktree').then(r => r.ok ? r.json() : null).catch(() => null),
        ]).then(([nextBranches, active]) => {
            if (cancelled) return;
            setBranches(nextBranches);
            setRepoBranch(active?.repoBranch ?? null);
            const resolved = resolveInitialBranch({
                serverActive: active?.branch ?? null,
            });
            if (resolved === defaultBranchRef.current) return;
            setDefaultBranch(resolved);
        });
        return () => { cancelled = true; };
    }, [devEnabled]);
    const refreshSyncStatus = useCallback(async () => {
        try {
            const nextStatus = await fetchSyncStatus();
            setSyncStatus(nextStatus);
            return nextStatus;
        } catch {
            return null;
        }
    }, []);
    useEffect(() => {
        refreshSyncStatus();
        const timer = window.setInterval(refreshSyncStatus, SYNC_STATUS_POLL_MS);
        return () => window.clearInterval(timer);
    }, [refreshSyncStatus]);
    useEffect(() => {
        fetchSyncProviders()
            .then(nextProviders => setSyncProviders(Array.isArray(nextProviders) ? nextProviders : []))
            .catch(() => {});
    }, []);
    const handleSidebarAction = useSidebarActions({
        devEnabled,
        checkForUpdate,
        beginSelfUpdate,
        failSelfUpdate,
        beginDevRecompile,
        failDevRecompile,
        defaultBranchRef,
    });
    const palette = usePaletteContext();
    useEffect(() => {
        palette.setActiveViewId(activeViewId);
        if (palette.active) palette.deactivate();
    }, [activeViewId]);
    const handleViewClick = useCallback((viewId) => {
        if (activePluginId) closePluginConfig();
        switchView(viewId);
    }, [activePluginId, closePluginConfig, switchView]);

    const globalCommands = useMemo(() => [
        ...viewOrder.map(id => ({
            id: `nav:${id}`,
            label: `Go to ${getViewLabel(id).text}`,
            run: () => switchView(id)
        })),
        { id: 'config:export', label: 'Export configuration', run: exportConfig },
        { id: 'config:import', label: 'Import configuration', run: importConfig },
    ], [viewOrder, switchView]);
    useRegisterCommands(GLOBAL_ID, globalCommands);

    return {
        devEnabled,
        appVersion,
        viewOrder,
        activeViewId,
        activePluginId,
        switchView,
        openPluginConfig,
        closePluginConfig,
        mounted,
        updateState,
        handleSidebarAction,
        handleViewClick,
        branches,
        repoBranch,
        defaultBranch,
        setDefaultBranch,
        syncStatus,
        syncProviders,
        setSyncStatus,
        refreshSyncStatus,
    };
}

async function exportConfig() {
    try {
        await exportProfile();
    } catch (error) {
        toast('error', `Failed to export profile: ${error.message}`);
    }
}

async function importConfig() {
    promptImportProfile({
        onImported: () => {
            toast('info', 'Profile imported. Reload the dashboard to refresh visible state.');
        },
        onError: (error) => {
            toast('error', `Failed to import profile: ${error.message}`);
        },
    });
}

function defaultSyncStatus() {
    return {
        configured: false,
        provider: null,
        provider_label: null,
        target_summary: null,
        health: 'not_configured',
        repo_url: null,
        folder_path: null,
        branch: null,
        path: null,
        commit_message: null,
        pull_on_launch: true,
        push_on_change: true,
        has_github_token: false,
        last_sync_at: null,
        incident: null,
        last_error: null,
        backups_dir: null,
        backup_count: 0,
        latest_backup_file: null,
    };
}
