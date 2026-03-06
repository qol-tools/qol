import { createLinkingActions } from './plugin-actions/linking.js';
import { createLogControlActions } from './plugin-actions/log-controls.js';
import { createReloadActions } from './plugin-actions/reload.js';

export function createPluginActionsController({
    state,
    discoveryController,
    getActivePluginBuildState,
    closePluginMenu,
    getMergedPluginById,
    onNeedsRender
}) {
    const reloadActions = createReloadActions({
        state,
        discoveryController,
        onNeedsRender
    });

    const linkingActions = createLinkingActions({
        state,
        discoveryController,
        getActivePluginBuildState,
        closePluginMenu,
        onNeedsRender,
        triggerReload: reloadActions.triggerReload
    });

    const logControlActions = createLogControlActions({
        state,
        discoveryController,
        getMergedPluginById,
        onNeedsRender
    });

    return {
        ...linkingActions,
        ...logControlActions,
        markReloadComplete: reloadActions.markReloadComplete,
        reloadPlugins: reloadActions.reloadPlugins
    };
}
