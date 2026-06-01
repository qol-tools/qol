export function declaredFieldsToSchema(itemFields) {
    const KIND_MAP = { 'string': 'string', 'number': 'number', 'boolean': 'boolean', 'string_array': 'string-array' };
    return Object.entries(itemFields).map(([key, kind]) => {
        if (kind === 'string_array' && key.endsWith('_mods')) return [key, 'mods'];
        return [key, KIND_MAP[kind] || 'string'];
    });
}
