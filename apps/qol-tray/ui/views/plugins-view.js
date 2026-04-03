import { html } from '../lib/html.js';
import { useMemo, useEffect, useRef } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../components/app/view-keyboard-context.js';

import { PageHeader } from '../components/PageHeader.js';
import { SurfaceContainer } from '../components/SurfaceContainer.js';
import { UninstallConfirmModal } from './plugins/confirm-modal.js';
import { PluginsGrid } from './plugins/grid.js';
import { usePluginsList } from './plugins/use-list.js';
import { usePluginsModal } from './plugins/use-modal.js';
import { usePluginActions } from './plugins/use-actions.js';
import { usePluginsKeyHandler } from './plugins/key-router.js';
import { useCardClickHandler } from './plugins/click-router.js';
import { matchesQuery, clampIndex } from '../utils/collections.js';


export function PluginsView({ onOpenPluginConfig, onOpenPluginUi }) {
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
    const modal = usePluginsModal(filtered);
    const actions = usePluginActions(filteredList, modal, onOpenPluginConfig, onOpenPluginUi);
    const handleKey = usePluginsKeyHandler(filteredList, modal, actions);
    useRegisterViewKeyboard('plugins', handleKey, actions.isBlocking);
    const handleCardClick = useCardClickHandler(filteredList, modal, actions);

    const modalRef = useRef(modal);
    modalRef.current = modal;
    const actionsRef = useRef(actions);
    actionsRef.current = actions;
    const commands = useMemo(() => [
        { id: 'plugins:uninstall', label: 'Uninstall selected plugin', run: () => { const p = filteredRef.current[list.selectedIndexRef.current]; if (p) modalRef.current.setConfirmPluginId(p.id); } },
        { id: 'plugins:update', label: 'Update selected plugin', run: () => { const p = filteredRef.current[list.selectedIndexRef.current]; if (p?.update_available) actionsRef.current.updatePlugin(p.id); } },
        { id: 'plugins:settings', label: 'Open plugin settings', run: () => actionsRef.current.openSelected() },
        { id: 'plugins:menu', label: 'Toggle context menu', run: () => modalRef.current.setContextMenuOpen(prev => !prev) },
    ], []);
    useRegisterCommands('plugins', commands);

    return html`<div class="view-container content-shell" onClick=${modal.closeAll}>
        <${PageHeader} title="Plugins" />
        <${SurfaceContainer} className="view-body">
            <${PluginsGrid}
                plugins=${filtered} ghostPlugins=${list.ghostPlugins}
                selectedIndex=${list.selectedIndex} contextMenuOpen=${modal.contextMenuOpen}
                updating=${actions.updating} onCardClick=${handleCardClick} onSelect=${list.setSelectedIndex} />
        <//>
        <${UninstallConfirmModal} plugin=${modal.confirmPlugin} pluginId=${modal.confirmPluginId}
            onClose=${modal.clearConfirm} onConfirm=${actions.confirmUninstall} />
    </div>`;
}
