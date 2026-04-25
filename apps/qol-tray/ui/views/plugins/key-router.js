import { useCallback } from 'preact/hooks';
import { withShiftVariants, dispatchKey } from '../../utils/keys.js';

function routePluginsKey(e, list, modal, actions) {
    if (modal.contextMenuOpenRef.current) {
        routeContextMenuKey(e, list, modal, actions);
        return;
    }
    routeNormalKey(e, list, modal, actions);
}

function routeContextMenuKey(e, list, modal, actions) {
    if (e.key === 'Escape' || (e.key === 'Enter' && e.shiftKey)) {
        e.preventDefault();
        modal.closeAll();
        actions.focusSelectedCard();
    }
}

function routeNormalKey(e, list, modal, actions) {
    if (e.key === 'Enter' && e.shiftKey) {
        e.preventDefault();
        modal.setContextMenuOpen(prev => !prev);
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

export function usePluginsKeyHandler(list, modal, actions) {
    return useCallback(
        e => routePluginsKey(e, list, modal, actions),
        [actions.openSelected, actions.focusSelectedCard, actions.navigateInGrid]
    );
}
