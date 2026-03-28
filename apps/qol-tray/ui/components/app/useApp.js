import { useCallback, useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { useRouter } from '../../hooks/useRouter.js';
import { usePaletteContext } from '../../palette/context.js';
import { useRegisterCommands } from '../../palette/useRegisterCommands.js';
import { GLOBAL_ID } from '../../palette/registry.js';
import { useAppBootstrap } from './useAppBootstrap.js';
import { useAppUpdateCoordinator } from './useAppUpdateCoordinator.js';
import { useMountedViews } from './useMountedViews.js';
import { useSidebarActions } from './useSidebarActions.js';
import { buildViewOrder, VIEW_LABELS } from './views.js';
import { exportProfile, promptImportProfile } from '../../views/profile/actions.js';
import { toast } from '../../lib/toast.js';

const WT_KEY = 'dev.recompile.defaultWorktree';

function readDefaultWorktree() {
    try { return localStorage.getItem(WT_KEY) || null; } catch { return null; }
}

export function useApp({ onDissolve } = {}) {
    const { devEnabled, appVersion } = useAppBootstrap();
    const viewOrder = useMemo(() => buildViewOrder(devEnabled), [devEnabled]);
    const { activeViewId, activePluginId, activePluginMode, switchView, openPluginConfig, openPluginUi, closePluginConfig } = useRouter({ viewOrder });
    const mounted = useMountedViews(activeViewId);
    const { updateState, checkForUpdate, beginSelfUpdate, failSelfUpdate, beginDevRecompile, failDevRecompile } = useAppUpdateCoordinator({ devEnabled, appVersion, onDissolve });
    const [worktrees, setWorktrees] = useState([]);
    const [defaultWorktree, setDefaultWorktreeState] = useState(readDefaultWorktree);
    const defaultWorktreeRef = useRef(defaultWorktree);
    defaultWorktreeRef.current = defaultWorktree;
    const setDefaultWorktree = useCallback(path => {
        const v = path || null;
        defaultWorktreeRef.current = v;
        try {
            if (v) localStorage.setItem(WT_KEY, v);
            if (!v) localStorage.removeItem(WT_KEY);
        } catch {}
        setDefaultWorktreeState(v);
    }, []);
    useEffect(() => {
        if (!devEnabled) return;
        fetch('/api/dev/worktrees')
            .then(r => r.ok ? r.json() : [])
            .then(nextWorktrees => {
                setWorktrees(nextWorktrees);
                const normalized = normalizeDefaultWorktree(defaultWorktreeRef.current, nextWorktrees);
                if (normalized === defaultWorktreeRef.current) return;
                setDefaultWorktree(normalized);
            })
            .catch(() => {});
    }, [devEnabled]);
    const handleSidebarAction = useSidebarActions({ checkForUpdate, beginSelfUpdate, failSelfUpdate, beginDevRecompile, failDevRecompile, defaultWorktreeRef });
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
            label: `Go to ${VIEW_LABELS[id] || id}`,
            run: () => switchView(id)
        })),
        { id: 'config:export', label: 'Export configuration', run: exportConfig },
        { id: 'config:import', label: 'Import configuration', run: importConfig },
    ], [viewOrder, switchView]);
    useRegisterCommands(GLOBAL_ID, globalCommands);

    return { devEnabled, appVersion, viewOrder, activeViewId, activePluginId, activePluginMode, switchView, openPluginConfig, openPluginUi, closePluginConfig, mounted, updateState, handleSidebarAction, handleViewClick, worktrees, defaultWorktree, setDefaultWorktree };
}

function normalizeDefaultWorktree(current, worktrees) {
    if (!current) return null;
    if (worktrees.some(worktree => worktree.path === current)) return current;
    const resolved = worktrees.find(worktree => parentDir(worktree.path) === current);
    if (resolved) return resolved.path;
    return null;
}

function parentDir(path) {
    const separator = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
    if (separator <= 0) return null;
    return path.slice(0, separator);
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
