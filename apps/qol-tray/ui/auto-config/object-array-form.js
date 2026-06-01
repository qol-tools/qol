function fieldPrefix(key) {
    const i = key.indexOf('_');
    if (i < 0) return null;
    return key.slice(0, i);
}

export function groupFields(schema) {
    const fromFields = [];
    const toFields = [];
    const booleans = [];
    const rest = [];

    for (const [key, type] of schema) {
        if (type === 'boolean') { booleans.push([key, type]); continue; }
        const prefix = fieldPrefix(key);
        if (prefix === 'from') { fromFields.push([key, type]); continue; }
        if (prefix === 'to') { toFields.push([key, type]); continue; }
        rest.push([key, type]);
    }

    return { fromFields, toFields, rest, booleans };
}
