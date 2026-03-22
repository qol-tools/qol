import { useCallback } from 'preact/hooks';

export function useGridNav(selector, selectedIndexRef, setSelectedIndex) {
    return useCallback((direction) => {
        const current = selectedIndexRef.current;
        const next = navigateGrid(selector, current, direction);
        if (next === current) return;
        setSelectedIndex(next);
        const card = document.querySelector(`${selector}[data-index="${next}"]`);
        if (card) card.focus();
    }, [selector, setSelectedIndex]);
}

export function navigateGrid(selector, selectedIndex, direction) {
    const model = gridSelectionModel(selector, selectedIndex);
    if (!model) return selectedIndex;

    const { rows, row, col } = model;
    switch (direction) {
        case 'left':
            return col > 0 ? rows[row][col - 1] : selectedIndex;
        case 'right':
            return col + 1 < rows[row].length ? rows[row][col + 1] : selectedIndex;
        case 'up': {
            if (row <= 0) return selectedIndex;
            return rows[row - 1][Math.min(col, rows[row - 1].length - 1)];
        }
        case 'down': {
            if (row + 1 >= rows.length) return selectedIndex;
            return rows[row + 1][Math.min(col, rows[row + 1].length - 1)];
        }
        default:
            return selectedIndex;
    }
}

export function useListNav(total, selectedIndex, setSelectedIndex) {
    return useCallback((delta) => {
        if (total === 0) return;
        setSelectedIndex(i => Math.max(0, Math.min(total - 1, i + delta)));
    }, [total, setSelectedIndex]);
}

function gridSelectionModel(selector, selectedIndex) {
    const rows = gridRows(selector);
    if (rows.length === 0) return null;
    const row = rows.findIndex(cols => cols.includes(selectedIndex));
    if (row < 0) return null;
    const col = rows[row].indexOf(selectedIndex);
    return { rows, row, col };
}

function gridRows(selector) {
    const cards = Array.from(document.querySelectorAll(selector))
        .map(card => ({
            index: parseInt(card.dataset.index ?? '', 10),
            top: card.offsetTop,
        }))
        .filter(card => !Number.isNaN(card.index));

    const rows = [];
    for (const card of cards) {
        const lastRow = rows[rows.length - 1];
        const lastCard = lastRow?.[lastRow.length - 1];
        if (!lastRow || Math.abs(card.top - lastCard.top) > 1) {
            rows.push([card]);
            continue;
        }
        lastRow.push(card);
    }

    return rows.map(row => row.map(card => card.index));
}
