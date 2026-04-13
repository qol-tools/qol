import { setVal } from './config-paths.js';

const RUNTIME_ONLY_KINDS = new Set(['action', 'list', 'status', 'qr_code']);

export function configFromForm(form, existingConfig = {}) {
    const base = cloneValue(existingConfig);
    const allFields = [...form.fields, ...form.sections.flatMap(section => section.fields)];
    return allFields.reduce((config, field) => {
        if (RUNTIME_ONLY_KINDS.has(field.kind)) return config;
        setConfigValue(config, field, field.value);
        return config;
    }, base);
}

export function getDisplaySections(form) {
    const root = form.fields.length > 0
        ? [{ id: '_root', label: 'General', description: '', fields: form.fields, actions: [] }]
        : [];
    return [...root, ...form.sections.filter(section => section.fields.length > 0)];
}

export function ownedConfigKeys(form) {
    const allFields = [...form.fields, ...form.sections.flatMap(section => section.fields)];
    const keys = new Set();
    for (const field of allFields) {
        if (RUNTIME_ONLY_KINDS.has(field.kind)) continue;
        const path = field.config_key || field.id;
        keys.add(path.split('.')[0]);
    }
    return keys;
}

function setConfigValue(config, field, value) {
    const path = field.config_key || field.id;
    setVal(config, path, cloneValue(value));
}

function cloneValue(value) {
    if (Array.isArray(value)) return value.map(cloneValue);
    if (value && typeof value === 'object') {
        return Object.fromEntries(
            Object.entries(value).map(([key, nested]) => [key, cloneValue(nested)])
        );
    }
    return value;
}
