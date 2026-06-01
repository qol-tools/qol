import { withShiftVariants, dispatchKey } from '../../utils/keys.js';

function activateSelected(actions) {
    const plugin = actions.filteredRef.current[actions.selectedIndexRef.current];
    if (!plugin || actions.isInstalling(plugin.id)) return;
    if (plugin.source === 'dev_linked') return;
    if (plugin.update_available) { actions.updatePlugin(plugin.id); return; }
    if (!plugin.installed) actions.installPlugin(plugin.id);
}

export function handleStoreKey(e, showTokenInputRef, actions) {
    if (showTokenInputRef.current && e.key === 'Escape') {
        e.preventDefault();
        actions.closeTokenInput();
        return;
    }
    handleGridKey(e, actions);
}

function handleGridKey(e, actions) {
    dispatchKey(e, withShiftVariants({
        ArrowUp: () => actions.navigateInGrid('up'),
        ArrowDown: () => actions.navigateInGrid('down'),
        ArrowLeft: () => actions.navigateInGrid('left'),
        ArrowRight: () => actions.navigateInGrid('right'),
        Enter: () => activateSelected(actions),
    }));
}
