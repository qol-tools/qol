export function handleDevKey(event, state, ctrl) {
    if (state.showLinkInput) return;
    if ((event.ctrlKey || event.metaKey) && (event.key === 'r' || event.key === 'R')) {
        event.preventDefault();
        void ctrl.actionsController.reloadPlugins();
        return;
    }
}
