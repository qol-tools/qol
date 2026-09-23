import {
    inferFieldType,
    isEmptyObjectArray,
    isModArray,
    isObjectArray,
    isPlainObject,
    isStringArray,
    prettyLabel,
} from './heuristics.js';

export function schemaTitle(pluginId, schema) {
    return schema?.title || `${prettyLabel(pluginId)} Settings`;
}

export function schemaDescription(schema) {
    return schema?.description || '';
}

export function fieldLabel(key, fieldSchema) {
    return fieldSchema?.label || prettyLabel(key);
}

export function fieldDescription(fieldSchema) {
    return fieldSchema?.description || '';
}

export function fieldPlaceholder(key, fieldSchema) {
    return fieldSchema?.placeholder || prettyLabel(key);
}

export function fieldOptions(fieldSchema) {
    return Array.isArray(fieldSchema?.options) ? fieldSchema.options : [];
}

export function resolveFieldSchema(schema, key) {
    return schema?.fields?.[key] || null;
}

export function orderedEntries(value, schema) {
    if (!schema?.fields || !isPlainObject(value)) {
        return Object.entries(value);
    }

    const entries = [];
    const seen = new Set();
    for (const key of Object.keys(schema.fields)) {
        if (!(key in value)) {
            continue;
        }
        entries.push([key, value[key]]);
        seen.add(key);
    }
    for (const [key, fieldValue] of Object.entries(value)) {
        if (seen.has(key)) {
            continue;
        }
        entries.push([key, fieldValue]);
    }
    return entries;
}

export function resolveFieldType(key, value, fieldSchema) {
    if (fieldSchema?.type) {
        return fieldSchema.type;
    }
    if (typeof value === 'boolean') {
        return 'boolean';
    }
    if (typeof value === 'number') {
        return 'number';
    }
    if (isModArray(key, value)) {
        return 'mods';
    }
    if (fieldOptions(fieldSchema).length > 0) {
        return 'select';
    }
    if (typeof value === 'string') {
        return 'string';
    }
    if (isStringArray(value)) {
        return 'string-array';
    }
    if (isObjectArray(value) || isEmptyObjectArray(key, value)) {
        return 'object-array';
    }
    if (isMapField(value, fieldSchema)) {
        return 'map';
    }
    if (isPlainObject(value)) {
        return 'object';
    }
    return inferFieldType(key, value);
}

export function isMapField(value, fieldSchema) {
    if (fieldSchema?.type === 'map') {
        return true;
    }
    if (fieldSchema?.additional_properties) {
        return true;
    }
    return false;
}

export function mapValueSchema(fieldSchema) {
    return fieldSchema?.value || fieldSchema?.additional_properties || null;
}

export function objectArrayFieldEntries(key, value, fieldSchema) {
    const schemaFields = fieldSchema?.item?.fields;
    if (schemaFields) {
        return Object.entries(schemaFields);
    }

    const inferred = value.length > 0 ? getObjectArraySchema(value) : guessSchemaFromKey(key);
    return Array.from(inferred.entries()).map(([fieldKey, type]) => [fieldKey, { type }]);
}

function getObjectArraySchema(arr) {
    const fields = new Map();
    for (const item of arr) {
        for (const [key, value] of Object.entries(item)) {
            if (fields.has(key)) {
                continue;
            }
            fields.set(key, inferFieldType(key, value));
        }
    }
    return fields;
}

function guessSchemaFromKey(key) {
    const fields = new Map();
    if (key.includes('key_rule') || key.includes('char_rule')) {
        fields.set('from_mods', 'mods');
        if (key.includes('char')) {
            fields.set('from_key', 'string');
            fields.set('to_char', 'string');
        }
        if (!key.includes('char')) {
            fields.set('to_mods', 'mods');
            fields.set('keys', 'string-array');
        }
        fields.set('global', 'boolean');
        return fields;
    }
    if (key.includes('mouse')) {
        fields.set('from_mods', 'mods');
        fields.set('button', 'string');
        fields.set('to_mods', 'mods');
        fields.set('global', 'boolean');
        return fields;
    }
    if (key.includes('scroll')) {
        fields.set('from_mods', 'mods');
        fields.set('to_mods', 'mods');
        fields.set('global', 'boolean');
        return fields;
    }
    fields.set('value', 'string');
    return fields;
}
