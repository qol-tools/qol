export function extractPath(value, path) {
    if (!path) return null;
    let current = value;
    for (const part of path.split('.')) {
        if (current == null || typeof current !== 'object') return null;
        current = current[part];
        if (current === undefined) return null;
    }
    return current;
}

export function queryFlag(value, path) {
    const flag = extractPath(value, path);
    return flag === true || flag === 1 || flag === 'true';
}

export function runtimeActivityLabel(field, value) {
    if (!queryFlag(value, field?.active_value_from)) return null;
    return field.active_label || 'Live';
}
