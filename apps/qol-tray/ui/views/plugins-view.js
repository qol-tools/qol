import { html } from '../lib/html.js';
import { useMemo, useEffect, useRef, useCallback } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../app/view-keyboard-context.js';

import { SurfaceContainer } from '../lib/components/SurfaceContainer.js';
import { PluginsGrid } from './plugins/grid.js';
import { usePluginsList } from './plugins/use-list.js';
import { usePluginsModal } from './plugins/use-modal.js';
import { usePluginActions } from './plugins/use-actions.js';
import { usePluginsKeyHandler } from './plugins/key-router.js';
import { useCardClickHandler } from './plugins/click-router.js';
import { matchesQuery, clampIndex } from '../utils/collections.js';


export function PluginsView({ onOpenPluginConfig }) {
    const { searchQuery } = usePaletteContext();
    const list = usePluginsList();
    const filtered = useMemo(
        () => searchQuery ? list.plugins.filter(p => matchesQuery([p?.name, p?.description], searchQuery)) : list.plugins,
        [list.plugins, searchQuery]
    );
    const filteredRef = useRef(filtered);
    filteredRef.current = filtered;
    useEffect(() => {
        list.setSelectedIndex(prev => clampIndex(prev, filtered.length));
    }, [filtered.length, list.setSelectedIndex]);
    const filteredList = { ...list, plugins: filtered, pluginsRef: filteredRef };
    const modal = usePluginsModal(filtered, list.refreshPlugins);
    const actions = usePluginActions(filteredList, modal, onOpenPluginConfig);
    const handleKey = usePluginsKeyHandler(actions);
    useRegisterViewKeyboard('plugins', handleKey, actions.isBlocking);
    const handleCardClick = useCardClickHandler(filteredList, actions);

    const actionsRef = useRef(actions);
    actionsRef.current = actions;
    const modalRef = useRef(modal);
    modalRef.current = modal;
    const handleToggleMenu = useCallback((index) => {
        list.setSelectedIndex(index);
        const plugin = filteredRef.current[index];
        if (plugin) actionsRef.current.openActionsMenu(plugin.id);
    }, [list.setSelectedIndex]);
    const commands = useMemo(() => [
        { id: 'plugins:uninstall', label: 'Uninstall selected plugin', run: () => { const p = filteredRef.current[list.selectedIndexRef.current]; if (p) modalRef.current.triggerUninstallConfirm(p.id); } },
        { id: 'plugins:update', label: 'Update selected plugin', run: () => { const p = filteredRef.current[list.selectedIndexRef.current]; if (p?.update_available) actionsRef.current.updatePlugin(p.id); } },
        { id: 'plugins:settings', label: 'Open plugin settings', run: () => actionsRef.current.openSelected() },
        { id: 'plugins:menu', label: 'Open plugin actions menu', run: () => actionsRef.current.openActionsMenu() },
    ], []);
    useRegisterCommands('plugins', commands);

    return html`<div class="view-container content-shell">
        <${SurfaceContainer} className="view-body">
            <${PluginsGrid}
                plugins=${filtered} ghostPlugins=${list.ghostPlugins}
                selectedIndex=${list.selectedIndex} loaded=${list.loaded}
                updating=${actions.updating} onCardClick=${handleCardClick} onSelect=${list.setSelectedIndex}
                onToggleMenu=${handleToggleMenu} />
        <//>
    </div>`;
}
