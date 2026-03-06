import { withShiftVariants, dispatchKey } from '../../utils/keys.js';

export function handleStoreKey(e, searchRef, showTokenInputRef, actions) {
    if (document.activeElement === searchRef.current) {
        handleSearchKey(e, searchRef, actions.refreshPlugins);
        return;
    }
    if (showTokenInputRef.current && e.key === 'Escape') {
        e.preventDefault();
        actions.closeTokenInput();
        return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 'r') {
        e.preventDefault();
        actions.refreshPlugins();
        return;
    }
    handleGridKey(e, actions);
}

function handleSearchKey(e, searchRef, refreshPlugins) {
    if (e.key === 'Escape') {
        e.preventDefault();
        searchRef.current.blur();
        return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 'r') {
        e.preventDefault();
        refreshPlugins();
    }
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
        s: () => actions.searchRef.current?.focus(),
        t: actions.openTokenInput
    }));
}
