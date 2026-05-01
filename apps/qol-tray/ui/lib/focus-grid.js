const ROW_ALIGNMENT_THRESHOLD = 4;

export function focusGridRows(elements) {
    const positioned = elements
        .map(element => {
            const rect = element.getBoundingClientRect();
            return { element, top: rect.top, left: rect.left };
        })
        .sort((a, b) => {
            if (Math.abs(a.top - b.top) > ROW_ALIGNMENT_THRESHOLD) return a.top - b.top;
            return a.left - b.left;
        });

    const rows = [];
    for (const item of positioned) {
        const row = rows[rows.length - 1];
        if (!row || Math.abs(row[0].top - item.top) > ROW_ALIGNMENT_THRESHOLD) {
            rows.push([item]);
        } else {
            row.push(item);
        }
    }
    return rows;
}

export function nextFocusGridElement(rows, active, direction) {
    if (rows.length === 0) return null;

    const position = findFocusGridPosition(rows, active);
    if (!position) return rows[0][0]?.element || null;

    const currentRow = rows[position.row];
    if (direction === 'left') return currentRow[Math.max(0, position.column - 1)]?.element || null;
    if (direction === 'right') return currentRow[Math.min(currentRow.length - 1, position.column + 1)]?.element || null;
    if (direction === 'up') {
        const previousRow = rows[position.row - 1];
        if (!previousRow) return currentRow[position.column]?.element || null;
        return previousRow[Math.min(position.column, previousRow.length - 1)]?.element || null;
    }
    const nextRow = rows[position.row + 1];
    if (!nextRow) return currentRow[position.column]?.element || null;
    return nextRow[Math.min(position.column, nextRow.length - 1)]?.element || null;
}

function findFocusGridPosition(rows, active) {
    for (let rowIndex = 0; rowIndex < rows.length; rowIndex += 1) {
        const columnIndex = rows[rowIndex].findIndex(item => item.element === active);
        if (columnIndex < 0) continue;
        return { row: rowIndex, column: columnIndex };
    }
    return null;
}
