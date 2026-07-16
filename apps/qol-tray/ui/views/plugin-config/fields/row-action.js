export function visibleRowAction(rowAction, row) {
    if (!rowAction || !rowAction.action) return null;
    if (rowAction.when && !row?.[rowAction.when]) return null;
    return { ...rowAction, label: rowAction.label || 'Run' };
}

export function visibleFieldRowAction(field, row) {
    return visibleFieldRowActions(field, row)[0] ?? null;
}

export function visibleFieldRowActions(field, row) {
    const actions = [field?.row_action, ...(field?.row_actions || [])].filter(Boolean);
    return actions.map(action => visibleRowAction(action, row)).filter(Boolean);
}

export function rowActionInput(rowAction, row) {
    return Object.fromEntries(
        Object.entries(rowAction?.input || {}).map(([key, template]) => [
            key,
            interpolateRowValue(template, row),
        ]),
    );
}

function interpolateRowValue(template, row) {
    if (typeof template !== 'string') return template;
    const exact = template.match(/^\{(\w+)\}$/);
    if (exact) return row?.[exact[1]] ?? null;
    return template.replace(/\{(\w+)\}/g, (_, key) => String(row?.[key] ?? ''));
}
