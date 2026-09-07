import { getVal, setVal } from './config-paths.js';

export function buildFieldPathIndex(form) {
    const paths = {};
    for (const field of form.fields) {
        paths[field.id] = field.config_key || field.id;
    }
    for (const section of form.sections) {
        for (const field of section.fields) {
            paths[field.id] = field.config_key || field.id;
        }
    }
    return paths;
}

export function getFieldValue(state, field) {
    return getVal(state.config, field.config_key || field.id);
}

export function setFieldValue(state, field, value) {
    setVal(state.config, field.config_key || field.id, value);
}

export function getFieldValueById(state, fieldId) {
    const path = state.fieldPaths?.[fieldId] || fieldId;
    return getVal(state.config, path);
}
