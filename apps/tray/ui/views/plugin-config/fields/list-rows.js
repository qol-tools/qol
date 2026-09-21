import { visibleFieldRowActions } from './row-action.js';

export function rowsFrom(data) {
    if (Array.isArray(data)) return data;
    if (data && Array.isArray(data.items)) return data.items;
    return [];
}

export function interpolate(template, row) {
    if (!template || !row) return '';
    return template.replace(/\{(\w+)\}/g, (_, key) => {
        const value = row[key];
        return value == null ? '' : String(value);
    });
}

export function listItem(field, row, index) {
    const actionPending = row.action_pending === true;
    const rowActions = visibleFieldRowActions(field, row);
    const actions = rowActions.map(rowAction => ({
        id: rowAction.action,
        label: rowAction.label,
        disabled: actionPending,
        rowAction,
    }));
    const primaryAction = actions[0];
    const hasRowActions = Boolean(field.row_action || field.row_actions?.length);
    const id = row.id ?? row.address ?? index;
    return {
        accent: row.accent,
        badge: row.badge,
        badgeTone: row.badge_tone,
        id: String(id),
        label: interpolate(field.row_label, row),
        description: interpolate(field.row_subtitle, row),
        actionLabel: primaryAction?.label,
        actions,
        disabled: actionPending || (hasRowActions && !primaryAction),
        keywords: [String(row.address ?? '')],
        row,
        pending: actionPending,
        primaryAction,
    };
}
