export function selectOptions(field, current, rows) {
    const options = [...(field.options || [])];
    const labels = { ...(field.option_labels || {}) };
    if (field.query) {
        for (const key of Object.keys(labels)) {
            if (!options.includes(key)) options.push(key);
        }
    }
    for (const row of Array.isArray(rows) ? rows : []) {
        const value = typeof row?.value === 'string' ? row.value : null;
        if (!value) continue;
        if (!options.includes(value)) options.push(value);
        if (typeof row.label === 'string' && !(value in labels)) labels[value] = row.label;
    }
    if (typeof current === 'string' && current && !options.includes(current)) {
        options.unshift(current);
    }
    return { options, labels };
}
