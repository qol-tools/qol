const ITEMS = [
    {
        id: 'update',
        label: 'Update',
        className: 'context-update',
        requires: 'update_available',
        handler: ({ actions }, pluginId) => {
            actions.updatePlugin(pluginId);
            actions.focusSelectedCard();
        },
    },
    {
        id: 'config',
        label: 'Config',
        className: 'context-config',
        requires: 'has_config',
        handler: ({ actions }) => actions.openConfig(),
    },
    {
        id: 'delete',
        label: 'Delete',
        className: 'context-delete',
        requires: null,
        handler: ({ modal }, pluginId) => modal.triggerUninstallConfirm(pluginId),
    },
];

export function pluginContextMenuItems(plugin) {
    if (!plugin) return [];
    return ITEMS.filter(item => item.requires == null || !!plugin[item.requires])
        .map(({ id, label, className }) => ({ id, label, className }));
}

export function dispatchPluginContextAction(actionId, pluginId, ctx) {
    const item = ITEMS.find(i => i.id === actionId);
    if (!item) return false;
    item.handler(ctx, pluginId);
    return true;
}

export function bindPluginContextMenuItems(plugin, ctx) {
    if (!plugin) return [];
    return ITEMS
        .filter(item => item.requires == null || !!plugin[item.requires])
        .map(item => ({
            id: item.id,
            label: item.label,
            run: () => item.handler(ctx, plugin.id),
        }));
}
