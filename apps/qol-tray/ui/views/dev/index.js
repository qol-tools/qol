import { html } from '../../lib/html.js';
import { useEffect, useRef, useMemo } from 'preact/hooks';
import { useRegisterCommands } from '../../palette/useRegisterCommands.js';

import { useDevController } from './use-controller.js';
import { DevLayout } from './components/DevLayout.js';

export const id = 'dev';

function useBuildOverlaySync(ctrl) {
    useEffect(() => {
        ctrl.buildController.cacheRows();
        ctrl.buildController.syncAll();
    });
}

export function DevViewInner() {
    const containerRef = useRef(null);
    const ctrl = useDevController(containerRef);

    useBuildOverlaySync(ctrl);

    const ctrlRef = useRef(ctrl);
    ctrlRef.current = ctrl;
    const commands = useMemo(() => [
        { id: 'dev:discover', label: 'Discover plugins', run: () => ctrlRef.current.triggerDiscovery() },
        { id: 'dev:reload', label: 'Reload plugins', run: () => ctrlRef.current.reloadPlugins() },
        { id: 'dev:menu', label: 'Toggle plugin menu', run: () => ctrlRef.current.handleItemActivation() },
    ], []);
    useRegisterCommands('dev', commands);

    return html`<${DevLayout} ctrl=${ctrl} containerRef=${containerRef} />`;
}
