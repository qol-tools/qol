export function selectedActionName(field, runtimeActive) {
    if (runtimeActive && field.active_action) return field.active_action;
    return field.action;
}

export function actionShowsActivity(field, runtimeActive) {
    return runtimeActive && field.variant !== 'toggle';
}

export function actionLabel(field, busy, runtimeActive, pairing) {
    if (busy) return 'Working...';
    if (field.active_action && runtimeActive) return field.active_label || 'Stop';
    if (field.action === 'pair' && pairing) return 'Stop Pairing';
    return field.label || 'Run';
}
