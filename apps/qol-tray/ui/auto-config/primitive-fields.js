import { getVal, setVal } from './config-paths.js';
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
    const div = document.createElement('div');
    div.className = 'field-group';

    const label = document.createElement('div');
    label.className = 'field-label';
    label.textContent = prettyLabel(key);
    div.appendChild(label);

    const isFloat = !Number.isInteger(value);
    const row = document.createElement('div');
    row.className = 'slider-row';

    if (isFloat && value >= 0 && value <= 1) {
        const slider = document.createElement('input');
        slider.type = 'range';
        slider.min = '0';
        slider.max = '1';
        slider.step = '0.01';
        slider.value = getVal(state.config, path);

        const valueSpan = document.createElement('span');
        valueSpan.className = 'slider-val';
        valueSpan.textContent = Number(getVal(state.config, path)).toFixed(2);

        slider.addEventListener('input', () => {
            const nextValue = parseFloat(slider.value);
            setVal(state.config, path, nextValue);
            valueSpan.textContent = nextValue.toFixed(2);
        });

        row.append(slider, valueSpan);
    } else {
        const input = document.createElement('input');
        input.type = 'number';
        input.className = 'number-input';
        input.value = getVal(state.config, path);
        if (isFloat) {
            input.step = 'any';
        }
        input.addEventListener('change', () => {
            const nextValue = isFloat ? parseFloat(input.value) : parseInt(input.value, 10);
            if (!Number.isNaN(nextValue)) {
                setVal(state.config, path, nextValue);
            }
        });
        row.appendChild(input);
    }

    div.appendChild(row);
    return div;
}

export function renderString(key, path, state) {
    const div = document.createElement('div');
    div.className = 'field-group';

    const label = document.createElement('div');
    label.className = 'field-label';
    label.textContent = prettyLabel(key);
    div.appendChild(label);

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

    const label = document.createElement('div');
    label.className = 'field-label';
    label.textContent = prettyLabel(key);
    div.appendChild(label);

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

    const label = document.createElement('div');
    label.className = 'field-label';
    label.textContent = prettyLabel(key);
    div.appendChild(label);
    div.appendChild(createModToggles(path, getVal(state.config, path) || [], state));

    return div;
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
