import { withShiftVariants, dispatchKey } from '../../utils/keys.js';

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
        Enter: () => {
            const plugin = actions.filteredRef.current[actions.selectedIndexRef.current];
            if (plugin && !plugin.installed && !actions.isInstalling(plugin.id)) {
                actions.installPlugin(plugin.id);
            }
        },
    }));
}
