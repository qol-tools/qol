export function updateSelection(selector, index) {
    document.querySelectorAll(selector).forEach((el, i) => {
        el.classList.toggle('selected', i === index);
    });
    const selected = document.querySelector(`${selector}.selected`);
    if (selected) {
        selected.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    }
}

export function navigate(state, key, total, delta) {
    if (total === 0) return false;
    const newIndex = Math.max(0, Math.min(total - 1, state[key] + delta));
    if (newIndex !== state[key]) {
        state[key] = newIndex;
        return true;
    }
    return false;
}

export function navigateGrid(selector, selectedIndex, direction) {
    const model = gridSelectionModel(selector, selectedIndex);
    if (!model) return selectedIndex;

    const { rows, row, col } = model;
    switch (direction) {
        case 'left': {
            const targetCol = col - 1;
            return targetCol >= 0 ? rows[row][targetCol] : selectedIndex;
        }
        case 'right': {
            const targetCol = col + 1;
            return targetCol < rows[row].length ? rows[row][targetCol] : selectedIndex;
        }
        case 'up': {
            const targetRow = row - 1;
            if (targetRow < 0) return selectedIndex;
            const targetCol = Math.min(col, rows[targetRow].length - 1);
            return rows[targetRow][targetCol];
        }
        case 'down': {
            const targetRow = row + 1;
            if (targetRow >= rows.length) return selectedIndex;
            const targetCol = Math.min(col, rows[targetRow].length - 1);
            return rows[targetRow][targetCol];
        }
        default:
            return selectedIndex;
    }
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
            index: Number.parseInt(card.dataset.index ?? '', 10),
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
