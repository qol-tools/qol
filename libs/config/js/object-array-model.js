const DECLARED_KIND_TYPES = {
    string: 'string',
    number: 'number',
    boolean: 'boolean',
    string_array: 'string-array',
};

function fieldPrefix(key) {
    const index = key.indexOf('_');
    if (index < 0) return null;
    return key.slice(0, index);
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

export function declaredFieldsToSchema(itemFields) {
    return Object.entries(itemFields).map(([key, kind]) => {
        if (kind === 'string_array' && key.endsWith('_mods')) return [key, 'mods'];
        return [key, DECLARED_KIND_TYPES[kind] || 'string'];
    });
}
