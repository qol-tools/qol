const SELECTOR_FONT = '600 13.12px "Segoe UI", "SF Pro Text", -apple-system, sans-serif';

export function collectVariantGroups(fields) {
    return fields
        .filter(field => isVariantSelector(field, fields))
        .map(selector => ({
            selector,
            fields: fields.filter(field => field.show_when?.field === selector.id),
        }));
}

export function buildBranchOwnerMap(groups) {
    return new Map(groups.flatMap(group => group.fields.map(field => [field.id, group])));
}

export function isFieldVisible(field, getFieldValueById) {
    if (!field.show_when) return true;
    return fieldValueEquals(getFieldValueById(field.show_when.field), field.show_when.equals);
}

export function optionLabel(field, option) {
    return field.option_labels?.[option] || option;
}

export function selectorDensityClass(field) {
    const maxLength = Math.max(...field.options.map(option => optionLabel(field, option).length));
    if (maxLength > 16) return 'variant-selector-tight';
    if (maxLength > 11) return 'variant-selector-compact';
    return '';
}

export function selectorGridTemplate(field) {
    const width = measureTextWidth(longestOptionLabel(field), SELECTOR_FONT);
    return `repeat(${field.options.length}, ${Math.ceil(width + 32)}px)`;
}

function isVariantSelector(field, fields) {
    if (field.kind !== 'select') return false;
    return fields.some(candidate => candidate.show_when?.field === field.id);
}

function fieldValueEquals(value, expected) {
    if (!Array.isArray(expected)) return value === expected;
    if (!Array.isArray(value)) return false;
    return JSON.stringify(value) === JSON.stringify(expected);
}

function longestOptionLabel(field) {
    return field.options.reduce((current, option) => {
        const label = optionLabel(field, option);
        return label.length > current.length ? label : current;
    }, '');
}

function measureTextWidth(text, font) {
    const canvas = document.createElement('canvas');
    const context = canvas.getContext('2d');
    if (!context) return text.length * 8;
    context.font = font;
    return context.measureText(text).width;
}
