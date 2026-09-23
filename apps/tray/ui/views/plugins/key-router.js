import { useCallback } from 'preact/hooks';
import { withShiftVariants, dispatchKey } from '../../utils/keys.js';

function focusedSurfaceOutsideGrid() {
    const active = document.activeElement;
    if (!(active instanceof HTMLElement) || active === document.body) return false;
    const surface = active.closest('[data-selected-surface]');
    return Boolean(surface) && !surface.closest('#plugins-grid');
}

function routePluginsKey(e, actions) {
    if (focusedSurfaceOutsideGrid()) return;
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
