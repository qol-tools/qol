export function nextSelectedIndex(selectedIndex, total, delta) {
    if (selectedIndex < 0) {
        return delta > 0 ? 0 : total - 1;
    }
    if (delta > 0) {
        return Math.min(selectedIndex + 1, total - 1);
    }
    return Math.max(selectedIndex - 1, 0);
}

function handleEscapeKey(event, state, bump) {
    if (!state.openPluginMenuId && !state.openCoreMenuId) return;
    event.preventDefault();
    state.openPluginMenuId = null;
    state.openCoreMenuId = null;
    bump();
}

function handleArrowKey(event, state, delta, bump) {
    const total = state.mergedCount || 0;
    if (total <= 0) return;
    event.preventDefault();
    state.selectedIndex = nextSelectedIndex(state.selectedIndex, total, delta);
    bump();
    const el = document.querySelector(`.plugin-list [data-selected-surface][data-index="${state.selectedIndex}"]`);
    if (el) el.focus();
}

export function handleDevKey(event, state, ctrl, bump) {
    if (state.showLinkInput) return;
    if ((event.ctrlKey || event.metaKey) && (event.key === 'r' || event.key === 'R')) {
        event.preventDefault();
        void ctrl.actionsController.reloadPlugins();
        return;
    }
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    if (event.key === 'Escape') { handleEscapeKey(event, state, bump); return; }
    if (event.key === 'ArrowDown') { handleArrowKey(event, state, 1, bump); return; }
    if (event.key === 'ArrowUp') { handleArrowKey(event, state, -1, bump); return; }
    if (event.key === ' ' || event.key === 'Enter') { event.preventDefault(); ctrl.actionsController.handleItemActivation(); return; }
}
