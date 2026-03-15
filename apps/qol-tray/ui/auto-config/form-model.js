import { setVal } from './config-paths.js';

export function configFromForm(form) {
    const allFields = [...form.fields, ...form.sections.flatMap(section => section.fields)];
    return allFields.reduce((config, field) => {
        setConfigValue(config, field, field.value);
        return config;
    }, {});
}

export function getDisplaySections(form) {
    const root = form.fields.length > 0
        ? [{ id: '_root', label: 'General', description: '', fields: form.fields, actions: [] }]
        : [];
    return [...root, ...form.sections.filter(section => section.fields.length > 0)];
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
