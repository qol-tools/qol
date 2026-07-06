export function isSliderNumberField(field) {
    if (field.kind !== 'number') {
        return false;
    }
    if (field.variant === 'slider') {
        return true;
    }
    return field.number?.min === 0 && field.number?.max === 1;
}

export function isActionRuntimeGated(field, runtimeDisabled) {
    if (!runtimeDisabled) {
        return false;
    }
    return !isActionRuntimeExempt(field);
}

function isActionRuntimeExempt(field) {
    return field?.variant === 'ghost' || field?.action === 'reload';
}
