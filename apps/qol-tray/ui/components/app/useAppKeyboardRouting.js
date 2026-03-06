import { useCallback } from 'preact/hooks';
import { useKeyboard } from '../../hooks/useKeyboard.js';
import { VIEW_MAP } from './views.js';

export function useAppKeyboardRouting({
    activePluginId,
    activeViewId,
    closePluginConfig,
    switchView,
    viewOrder
}) {
    const cycleView = useCallback((event) => {
        event.preventDefault();
        const idx = viewOrder.indexOf(activeViewId);
        const next = event.shiftKey
            ? (idx - 1 + viewOrder.length) % viewOrder.length
            : (idx + 1) % viewOrder.length;
        switchView(viewOrder[next]);
    }, [activeViewId, switchView, viewOrder]);

    useKeyboard(useCallback((event) => {
        if (activePluginId) {
            if (event.key === 'Escape') {
                event.preventDefault();
                closePluginConfig();
                return;
            }
            if (event.key === 'Tab') {
                closePluginConfig();
                cycleView(event);
            }
            return;
        }

        const view = VIEW_MAP[activeViewId];

        if (view?.isBlocking?.()) {
            if (view.handleKey) {
                view.handleKey(event);
            }
            return;
        }

        if (event.key === 'Tab') {
            cycleView(event);
            return;
        }

        if (view?.handleKey) {
            view.handleKey(event);
        }
    }, [activePluginId, activeViewId, closePluginConfig, cycleView]));
}
