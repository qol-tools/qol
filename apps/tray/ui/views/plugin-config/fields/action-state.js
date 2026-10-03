import { queryFlag } from './query-data.js';

export function selectedActionName(field, runtimeActive) {
    if (runtimeActive && field.active_action) return field.active_action;
    return field.action;
}

export function actionShowsActivity(field, runtimeActive) {
    return runtimeActive && field.variant !== 'toggle';
}

export function actionRuntimeState(field, queryState) {
    const unavailable = Boolean(field.active_query && queryState.error);
    return {
        unavailable,
        active: !unavailable && queryFlag(queryState.data, field.active_value_from),
    };
}

export function actionLabel(field, busy, runtimeActive, pairing, unavailable = false) {
    if (busy) return 'Working...';
    if (unavailable) return 'Unavailable';
    if (field.active_action && runtimeActive) return field.active_label || 'Stop';
    if (field.action === 'pair' && pairing) return 'Stop Pairing';
    return field.label || 'Run';
}
