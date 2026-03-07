import { html } from '../../lib/html.js';
import { useEffect, useRef } from 'preact/hooks';
import { useFooterShortcuts } from '../../hooks/useFooterShortcuts.js';
import { useDevController } from './use-controller.js';
import { DevLayout } from './components/DevLayout.js';

export const id = 'dev';

const SHORTCUTS = [
    { key: '↑↓', label: 'navigate' },
    { key: 'Enter', label: 'toggle' },
    { key: 'm', label: 'menu' },
    { key: 'Esc', label: 'close menu' },
    { key: 'r', label: 'discover' },
    { key: '⌘R', label: 'reload' }
];

function useBuildOverlaySync(ctrl) {
    useEffect(() => {
        ctrl.buildController.cacheRows();
        ctrl.buildController.syncAll();
    });
}

export function DevViewInner() {
    const containerRef = useRef(null);
    const ctrl = useDevController(containerRef);
    useFooterShortcuts(SHORTCUTS);
    useBuildOverlaySync(ctrl);
    DevViewInner.handleKey = ctrl.handleKey;
    DevViewInner.isBlocking = () => false;
    return html`
        <div ref=${containerRef} style="flex:1;min-height:0;display:flex;flex-direction:column">
            <${DevLayout} ctrl=${ctrl} />
        </div>
    `;
}
