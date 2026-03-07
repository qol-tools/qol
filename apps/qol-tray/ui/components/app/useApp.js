import { useCallback, useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { useRouter } from '../../hooks/useRouter.js';
import { usePaletteContext } from '../../palette/context.js';
import { useRegisterCommands } from '../../palette/useRegisterCommands.js';
import { GLOBAL_ID } from '../../palette/registry.js';
import { useAppBootstrap } from './useAppBootstrap.js';
import { useAppKeyboardRouting } from './useAppKeyboardRouting.js';
import { useAppUpdateCoordinator } from './useAppUpdateCoordinator.js';
import { useMountedViews } from './useMountedViews.js';
import { useSidebarActions } from './useSidebarActions.js';
import { buildViewOrder, VIEW_LABELS } from './views.js';

const WT_KEY = 'dev.recompile.defaultWorktree';

function readDefaultWorktree() {
    try { return localStorage.getItem(WT_KEY) || null; } catch { return null; }
}

export function useApp() {
    const { devEnabled, appVersion } = useAppBootstrap();
    const viewOrder = useMemo(() => buildViewOrder(devEnabled), [devEnabled]);
    const { activeViewId, activePluginId, switchView, openPluginConfig, closePluginConfig } = useRouter({ viewOrder });
    const mounted = useMountedViews(activeViewId);
    const { updateState, checkForUpdate, beginSelfUpdate, failSelfUpdate, beginDevRecompile, failDevRecompile } = useAppUpdateCoordinator({ devEnabled, appVersion });
    const [worktrees, setWorktrees] = useState([]);
    const [defaultWorktree, setDefaultWorktreeState] = useState(readDefaultWorktree);
    const defaultWorktreeRef = useRef(defaultWorktree);
    defaultWorktreeRef.current = defaultWorktree;
    const setDefaultWorktree = useCallback(path => {
        const v = path || null;
        defaultWorktreeRef.current = v;
        try { if (v) localStorage.setItem(WT_KEY, v); else localStorage.removeItem(WT_KEY); } catch {}
        setDefaultWorktreeState(v);
    }, []);
    useEffect(() => {
        if (!devEnabled) return;
        fetch('/api/dev/worktrees').then(r => r.ok ? r.json() : []).then(setWorktrees).catch(() => {});
    }, [devEnabled]);
    const handleSidebarAction = useSidebarActions({ checkForUpdate, beginSelfUpdate, failSelfUpdate, beginDevRecompile, failDevRecompile, defaultWorktreeRef });
    const palette = usePaletteContext();
    useEffect(() => { palette.setActiveViewId(activeViewId); }, [activeViewId]);
    useAppKeyboardRouting({ activePluginId, activeViewId, closePluginConfig, switchView, viewOrder, palette });
    const handleViewClick = useCallback((viewId) => {
        if (activePluginId) closePluginConfig();
        switchView(viewId);
    }, [activePluginId, closePluginConfig, switchView]);

    const globalCommands = useMemo(() =>
        viewOrder.map(id => ({
            id: `nav:${id}`,
            label: `Go to ${VIEW_LABELS[id] || id}`,
            run: () => switchView(id)
        })),
        [viewOrder, switchView]
    );
    useRegisterCommands(GLOBAL_ID, globalCommands);

    return { devEnabled, appVersion, viewOrder, activeViewId, activePluginId, openPluginConfig, closePluginConfig, mounted, updateState, handleSidebarAction, handleViewClick, worktrees, defaultWorktree, setDefaultWorktree };
}
