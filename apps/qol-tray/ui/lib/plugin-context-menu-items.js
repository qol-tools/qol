// Pure helper that derives the visible context-menu items for a plugin card.
// Kept separate from the view so it can be tested as a data-driven contract.
//
// The order here is the on-screen order. Visibility depends only on the plugin
// capability flags; Delete is always shown. Adding a new item means adding one
// row to the array — no new if/else branches, no new render code.
//
// Each item carries a `handler(ctx, pluginId)` that implements its action
// given a `ctx` of { actions, modal }. The caller dispatches via
// `items.find(i => i.id === id)?.handler?.(ctx, pluginId)` — adding a new
// menu item never requires editing the dispatcher.

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
        handler: ({ modal }, pluginId) => modal.setConfirmPluginId(pluginId),
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
