export const KNOWN_MODS = ['ctrl', 'shift', 'alt', 'cmd', 'ralt', 'altgr'];

export function prettyLabel(key) {
    return key
        .replace(/_/g, ' ')
        .replace(/-/g, ' ')
        .replace(/\b\w/g, c => c.toUpperCase());
}

export function isModArray(key, value) {
    if (!Array.isArray(value)) {
        return false;
    }
    if (value.length === 0) {
        return key.endsWith('_mods') || key === 'from_mods' || key === 'to_mods';
    }
    return value.every(v => typeof v === 'string' && KNOWN_MODS.includes(v));
}

export function isStringArray(value) {
    return Array.isArray(value) && value.every(v => typeof v === 'string');
}

export function isObjectArray(value) {
    return Array.isArray(value)
        && value.length > 0
        && typeof value[0] === 'object'
        && value[0] !== null
        && !Array.isArray(value[0]);
}

export function isEmptyObjectArray(key, value) {
    return Array.isArray(value) && value.length === 0 && key.endsWith('_rules');
}

export function isHexColor(value) {
    return typeof value === 'string' && /^[0-9a-f]{6}$/i.test(value);
}

export function isColorField(key, value) {
    return isHexColor(value) || (typeof value === 'string' && (key.endsWith('_color') || key.includes('color')));
}

export function isPlainObject(value) {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function inferFieldType(key, value) {
    if (typeof value === 'boolean') {
        return 'boolean';
    }
    if (typeof value === 'number') {
        return 'number';
    }
    if (isModArray(key, value)) {
        return 'mods';
    }
    if (typeof value === 'string') {
        return 'string';
    }
    if (isStringArray(value)) {
        return 'string-array';
    }
    return 'unknown';
}

export function getObjectArraySchema(arr) {
    const fields = new Map();
    for (const item of arr) {
        for (const [key, value] of Object.entries(item)) {
            if (!fields.has(key)) {
                fields.set(key, inferFieldType(key, value));
            }
        }
    }
    return fields;
}

export function guessSchemaFromKey(key) {
    const fields = new Map();
    if (key.includes('key_rule') || key.includes('char_rule')) {
        fields.set('from_mods', 'mods');
        if (key.includes('char')) {
            fields.set('from_key', 'string');
            fields.set('to_char', 'string');
        } else {
            fields.set('to_mods', 'mods');
            fields.set('keys', 'string-array');
        }
        fields.set('global', 'boolean');
    } else if (key.includes('mouse')) {
        fields.set('from_mods', 'mods');
        fields.set('button', 'string');
        fields.set('to_mods', 'mods');
        fields.set('global', 'boolean');
    } else if (key.includes('scroll')) {
        fields.set('from_mods', 'mods');
        fields.set('to_mods', 'mods');
        fields.set('global', 'boolean');
    } else {
        fields.set('value', 'string');
    }
    return fields;
}
