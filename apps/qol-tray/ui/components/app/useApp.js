import { useCallback, useMemo } from 'preact/hooks';
import { useRouter } from '../../hooks/useRouter.js';
import { useAppBootstrap } from './useAppBootstrap.js';
import { useAppKeyboardRouting } from './useAppKeyboardRouting.js';
import { useAppUpdateCoordinator } from './useAppUpdateCoordinator.js';
import { useMountedViews } from './useMountedViews.js';
import { useSidebarActions } from './useSidebarActions.js';
import { buildViewOrder } from './views.js';

export function useApp() {
    const { devEnabled, appVersion } = useAppBootstrap();
    const viewOrder = useMemo(() => buildViewOrder(devEnabled), [devEnabled]);
    const { activeViewId, activePluginId, switchView, openPluginConfig, closePluginConfig } = useRouter({ viewOrder });
    const mounted = useMountedViews(activeViewId);
    const { updateState, checkForUpdate, beginSelfUpdate, failSelfUpdate, beginDevRecompile, failDevRecompile } = useAppUpdateCoordinator({ devEnabled, appVersion });
    const handleSidebarAction = useSidebarActions({ checkForUpdate, beginSelfUpdate, failSelfUpdate, beginDevRecompile, failDevRecompile });
    useAppKeyboardRouting({ activePluginId, activeViewId, closePluginConfig, switchView, viewOrder });
    const handleViewClick = useCallback((viewId) => {
        if (activePluginId) closePluginConfig();
        switchView(viewId);
    }, [activePluginId, closePluginConfig, switchView]);
    return { devEnabled, appVersion, viewOrder, activeViewId, activePluginId, openPluginConfig, closePluginConfig, mounted, updateState, handleSidebarAction, handleViewClick };
}
