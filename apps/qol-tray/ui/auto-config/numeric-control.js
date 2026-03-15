import { getFieldValue, setFieldValue } from './normalized-config.js';

export function renderNumberField(field, state, createFieldGroup) {
    const group = createFieldGroup(field);
    group.classList.add('field-group-number');

    const control = document.createElement('div');
    control.className = 'number-control input-adornment';

    const input = document.createElement('input');
    input.type = 'number';
    input.className = 'number-input';
    input.value = `${getFieldValue(state, field)}`;
    if (field.number.min !== null) {
        input.min = `${field.number.min}`;
    }
    if (field.number.max !== null) {
        input.max = `${field.number.max}`;
    }
    if (field.number.step !== null) {
        input.step = `${field.number.step}`;
    }
    input.addEventListener('change', () => {
        syncNumberField(input, field, state);
        state.save();
    });

    control.appendChild(input);

    const unit = inferNumberUnit(field);
    if (unit) {
        control.appendChild(createNumberUnit(unit));
    }

    group.appendChild(control);
    return group;
}

function syncNumberField(input, field, state) {
    const raw = Number(input.value);
    if (Number.isNaN(raw)) {
        input.value = `${getFieldValue(state, field)}`;
        return;
    }
    const clamped = clampNumber(raw, field.number);
    setFieldValue(state, field, clamped);
    input.value = `${clamped}`;
}

function clampNumber(value, constraints) {
    if (constraints.min !== null && value < constraints.min) {
        return constraints.min;
    }
    if (constraints.max !== null && value > constraints.max) {
        return constraints.max;
    }
    return value;
}

function createNumberUnit(text) {
    const unit = document.createElement('span');
    unit.className = 'number-unit input-adornment__suffix';
    unit.textContent = text;
    return unit;
}

function inferNumberUnit(field) {
    if (field.id.endsWith('_percent')) {
        return '%';
    }
    if (field.id.endsWith('_px') || field.id.endsWith('_pixels')) {
        return 'px';
    }
    return '';
}
