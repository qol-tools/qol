import { useCallback } from 'preact/hooks';
import { withShiftVariants, dispatchKey } from '../../utils/keys.js';

function routePluginsKey(e, actions) {
    if (e.key === 'Enter' && e.shiftKey) {
        e.preventDefault();
        actions.openActionsMenu();
        return;
    }
    dispatchKey(e, withShiftVariants({
        ArrowUp: () => actions.navigateInGrid('up'),
        ArrowDown: () => actions.navigateInGrid('down'),
        ArrowLeft: () => actions.navigateInGrid('left'),
        ArrowRight: () => actions.navigateInGrid('right'),
        Enter: actions.openSelected,
    }));
}

export function usePluginsKeyHandler(actions) {
    return useCallback(
        e => routePluginsKey(e, actions),
        [actions.openSelected, actions.openActionsMenu, actions.navigateInGrid]
    );
}
