import { html } from '../lib/html.js';
import { useRef, useMemo } from 'preact/hooks';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useStoreController } from './store/use-controller.js';
import { StoreLayout } from './store/layout.js';

export function StoreView() {
    const ctrl = useStoreController();
    StoreView.handleKey = ctrl.handleKey;
    StoreView.isBlocking = () => false;

    const ctrlRef = useRef(ctrl);
    ctrlRef.current = ctrl;
    const commands = useMemo(() => [
        { id: 'store:install', label: 'Install selected plugin', run: () => {
            const c = ctrlRef.current;
            const p = c.filteredRef.current[c.selectedIndexRef.current];
            if (p && !p.installed && !c.isInstalling(p.id)) c.installPlugin(p.id);
        }},
        { id: 'store:token', label: 'Manage GitHub token', run: () => ctrlRef.current.openTokenInput() },
        { id: 'store:refresh', label: 'Refresh plugin store', run: () => ctrlRef.current.refreshPlugins() },
    ], []);
    useRegisterCommands('store', commands);

    return html`<${StoreLayout} ctrl=${ctrl} />`;
}
