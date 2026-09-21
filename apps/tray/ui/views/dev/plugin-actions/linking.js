import { createLinkingApiActions } from './linking/api-actions.js';
import { createLinkInputState } from './linking/input-state.js';

export function createLinkingActions({
    state,
    discoveryController,
    getActivePluginBuildState,
    closePluginMenu,
    onNeedsRender,
    triggerReload
}) {
    const linkInputState = createLinkInputState({
        state,
        onNeedsRender
    });

    const apiActions = createLinkingApiActions({
        state,
        discoveryController,
        onNeedsRender,
        triggerReload,
        linkInputState
    });

    function handleItemActivation() {
        const item = state.mergedList[state.selectedIndex];
        if (!item) {
            return;
        }

        closePluginMenu();
        if (getActivePluginBuildState(item)) {
            return;
        }

        if (item.status === 'linked') {
            void apiActions.deleteLink(item.id);
            return;
        }

        if (item.path) {
            void apiActions.quickLink(item.path, item.id);
            return;
        }

        linkInputState.showLinkInput();
    }

    return {
        cancelLink: linkInputState.cancelLink,
        confirmLink: apiActions.confirmLink,
        deleteLink: apiActions.deleteLink,
        handleItemActivation,
        quickLink: apiActions.quickLink,
        showLinkInput: linkInputState.showLinkInput
    };
}
