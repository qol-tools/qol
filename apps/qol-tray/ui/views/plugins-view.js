import { html } from '../lib/html.js';
import { useFeedback } from '../hooks/useFeedback.js';
import { useFooterShortcuts } from '../hooks/useFooterShortcuts.js';
import { Feedback } from '../components/FeedbackPreact.js';
import { PageHeader } from '../components/PageHeader.js';
import { UninstallConfirmModal } from './plugins/confirm-modal.js';
import { PluginsGrid } from './plugins/grid.js';
import { usePluginsList } from './plugins/use-list.js';
import { usePluginsModal } from './plugins/use-modal.js';
import { usePluginActions } from './plugins/use-actions.js';
import { usePluginsKeyHandler } from './plugins/key-router.js';
import { useCardClickHandler } from './plugins/click-router.js';

const SHORTCUTS = [
    { key: '←↑↓→', label: 'navigate' },
    { key: 'Enter', label: 'settings' },
    { key: 'u', label: 'update' },
    { key: 'd', label: 'delete' },
    { key: 'm', label: 'menu' }
];

export function PluginsView({ onOpenPluginConfig }) {
    const { feedback, setFeedback, clearFeedback } = useFeedback();
    const list = usePluginsList(setFeedback);
    const modal = usePluginsModal(list.plugins);
    const actions = usePluginActions(list, modal, setFeedback, clearFeedback, onOpenPluginConfig);
    useFooterShortcuts(SHORTCUTS);
    PluginsView.handleKey = usePluginsKeyHandler(list, modal, actions);
    PluginsView.isBlocking = actions.isBlocking;
    const handleCardClick = useCardClickHandler(list, modal, actions);
    return html`<div class="view-container" onClick=${modal.closeAll}>
        <${PageHeader} title="Plugins" />
        <div class="view-body">
            <${Feedback} feedback=${feedback} />
            <${PluginsGrid}
                plugins=${list.plugins} ghostPlugins=${list.ghostPlugins}
                selectedIndex=${list.selectedIndex} contextMenuOpen=${modal.contextMenuOpen}
                updating=${actions.updating} onCardClick=${handleCardClick} />
        </div>
        <${UninstallConfirmModal} plugin=${modal.confirmPlugin} pluginId=${modal.confirmPluginId}
            onClose=${modal.clearConfirm} onConfirm=${actions.confirmUninstall} />
    </div>`;
}
