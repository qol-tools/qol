export function visibleRowAction(rowAction, row) {
    if (!rowAction || !rowAction.action) return null;
    if (rowAction.when && !row?.[rowAction.when]) return null;
    return { action: rowAction.action, label: rowAction.label || 'Run' };
}

export function firstActionableRow(rowAction, rows) {
    return (rows || []).find((row) => visibleRowAction(rowAction, row)) ?? null;
}
