import { getVal, setVal } from './config-paths.js';
import { createFieldLabel } from './field-label.js';
import { KNOWN_MODS, prettyLabel } from './heuristics.js';

export function renderBoolean(key, path, state) {
    const div = document.createElement('div');
    div.className = 'field-group toggle-row';

    const label = document.createElement('label');
    const checkbox = document.createElement('input');
    checkbox.type = 'checkbox';
    checkbox.checked = getVal(state.config, path);
    checkbox.addEventListener('change', () => setVal(state.config, path, checkbox.checked));

    const strong = document.createElement('strong');
    strong.textContent = prettyLabel(key);

    label.append(checkbox, strong);
    div.appendChild(label);
    return div;
}

export function renderNumber(key, value, path, state) {
    return renderNumericField({ key, value, path, state });
}

function renderNumericField(field) {
    const div = document.createElement('div');
    div.className = 'field-group field-group-number';
    div.append(createFieldLabel(field.key), buildNumberControl(field));
    return div;
}

export function renderString(key, path, state) {
    const div = document.createElement('div');
    div.className = 'field-group';
    div.appendChild(createFieldLabel(key));

    const input = document.createElement('input');
    input.type = 'text';
    input.className = 'text-input';
    input.value = getVal(state.config, path);
    input.addEventListener('input', () => setVal(state.config, path, input.value));
    div.appendChild(input);

    return div;
}

export function renderColor(key, path, state) {
    const div = document.createElement('div');
    div.className = 'field-group';
    div.appendChild(createFieldLabel(key));

    const row = document.createElement('div');
    row.className = 'color-row';

    const swatch = document.createElement('input');
    swatch.type = 'color';
    swatch.className = 'color-swatch';
    swatch.value = `#${getVal(state.config, path) || '000000'}`;

    const hex = document.createElement('input');
    hex.type = 'text';
    hex.className = 'color-hex';
    hex.value = getVal(state.config, path) || '';

    swatch.addEventListener('input', () => {
        const nextValue = swatch.value.replace('#', '');
        hex.value = nextValue;
        setVal(state.config, path, nextValue);
    });

    hex.addEventListener('input', () => {
        const nextValue = hex.value.replace('#', '');
        if (/^[0-9a-f]{6}$/i.test(nextValue)) {
            swatch.value = `#${nextValue}`;
            setVal(state.config, path, nextValue);
        }
    });

    row.append(swatch, hex);
    div.appendChild(row);
    return div;
}

export function renderModArrayStandalone(key, path, state) {
    const div = document.createElement('div');
    div.className = 'field-group';
    div.appendChild(createFieldLabel(key));
    div.appendChild(createModToggles(path, getVal(state.config, path) || [], state));

    return div;
}

function buildNumberControl(field) {
    if (showsSlider(field.value)) return buildSliderControl(field);
    return buildStepperControl(field);
}

function showsSlider(value) {
    return !Number.isInteger(value) && value >= 0 && value <= 1;
}

function buildSliderControl(field) {
    const row = document.createElement('div');
    row.className = 'slider-row';
    const slider = createSliderInput(field.path, field.state);
    const value = createSliderValue(field.path, field.state);
    slider.addEventListener('input', () => syncSliderValue(slider, value, field.path, field.state));
    row.append(slider, value);
    return row;
}

function createSliderInput(path, state) {
    const slider = document.createElement('input');
    slider.type = 'range';
    slider.min = '0';
    slider.max = '1';
    slider.step = '0.01';
    slider.value = getVal(state.config, path);
    return slider;
}

function createSliderValue(path, state) {
    const value = document.createElement('span');
    value.className = 'slider-val';
    value.textContent = Number(getVal(state.config, path)).toFixed(2);
    return value;
}

function syncSliderValue(slider, value, path, state) {
    const nextValue = parseFloat(slider.value);
    setVal(state.config, path, nextValue);
    value.textContent = nextValue.toFixed(2);
}

function buildStepperControl(field) {
    const input = createNumberInput(field);
    input.dataset.wheelDelta = '0';
    input.addEventListener('wheel', event => handleNumberWheel(event, input, field), { passive: false });
    return input;
}

function createNumberInput(field) {
    const input = document.createElement('input');
    input.type = 'number';
    input.className = 'number-input';
    input.style.cursor = 'ns-resize';
    input.title = 'Scroll to adjust';
    input.value = getVal(field.state.config, field.path);
    if (!Number.isInteger(field.value)) input.step = 'any';
    input.addEventListener('change', () => syncNumberInput(input, field));
    return input;
}

function syncNumberInput(input, field) {
    const nextValue = parseInputValue(input.value, field.value);
    if (Number.isNaN(nextValue)) return;
    input.value = nextValue;
    setVal(field.state.config, field.path, nextValue);
}

function nudgeNumberInput(input, field, direction) {
    const current = parseInputValue(input.value, field.value);
    const step = !Number.isInteger(field.value) ? 0.1 : 1;
    input.value = current + direction * step;
    syncNumberInput(input, field);
}

function handleNumberWheel(event, input, field) {
    event.preventDefault();
    const direction = wheelDirection(event, input);
    if (!direction) return;
    nudgeNumberInput(input, field, direction);
}

function parseInputValue(value, fallback) {
    if (Number.isInteger(fallback)) return parseInt(value, 10);
    return parseFloat(value);
}

function wheelDirection(event, input) {
    const nextDelta = parseFloat(input.dataset.wheelDelta || '0') + event.deltaY;
    input.dataset.wheelDelta = `${nextDelta}`;
    if (Math.abs(nextDelta) < 45) return 0;
    input.dataset.wheelDelta = '0';
    return nextDelta < 0 ? 1 : -1;
}

export function createModToggles(path, activeMods, state) {
    const row = document.createElement('div');
    row.className = 'mod-toggles';
    row.dataset.path = path;

    for (const mod of KNOWN_MODS) {
        const chip = document.createElement('button');
        chip.type = 'button';
        chip.className = `mod-chip${activeMods.includes(mod) ? ' active' : ''}`;
        chip.textContent = mod;
        chip.dataset.mod = mod;
        chip.addEventListener('click', () => {
            chip.classList.toggle('active');
            const active = Array.from(row.querySelectorAll('.mod-chip.active')).map(button => button.dataset.mod);
            setVal(state.config, path, active);
        });
        row.appendChild(chip);
    }

    return row;
}

export function appendStaticModChips(container, mods) {
    for (const mod of mods) {
        const chip = document.createElement('span');
        chip.className = 'mod-chip-static';
        chip.textContent = mod;
        container.appendChild(chip);
    }
}
