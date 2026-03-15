import { getVal, setVal } from './config-paths.js';
import { createFieldLabel } from './field-label.js';
import { KNOWN_MODS, prettyLabel } from './heuristics.js';

export function buildAddForm(container, schema, arrayPath, state, rerender) {
    container.replaceChildren();
    const inputs = buildFieldInputs(container, schema);
    const addButton = document.createElement('button');
    addButton.type = 'button';
    addButton.className = 'btn btn-ghost btn-sm btn-add';
    addButton.textContent = '+ Add';
    addButton.addEventListener('click', () => handleAdd(inputs, arrayPath, state, rerender));
    container.appendChild(addButton);
}

function buildFieldInputs(container, schema) {
    const inputs = {};
    for (const [fieldKey, fieldType] of schema) {
        if (fieldType === 'mods') { inputs[fieldKey] = buildModsInput(container, fieldKey); continue; }
        if (fieldType === 'boolean') { inputs[fieldKey] = buildBooleanInput(container, fieldKey); continue; }
        if (fieldType === 'string-array') { inputs[fieldKey] = buildStringArrayInput(container, fieldKey); continue; }
        inputs[fieldKey] = buildScalarInput(container, fieldKey, fieldType);
    }
    return inputs;
}

function buildModsInput(container, fieldKey) {
    const toggles = buildModSide(container, fieldKey);
    if (fieldKey === 'from_mods') {
        const arrow = document.createElement('span');
        arrow.className = 'arrow';
        arrow.textContent = '→';
        container.appendChild(arrow);
    }
    return { type: 'mods', el: toggles };
}

function buildModSide(container, fieldKey) {
    const side = document.createElement('div');
    side.className = 'rule-side';
    const label = createFieldLabel(fieldKey);
    label.style.marginRight = '0.25rem';
    label.style.marginBottom = '0';
    side.appendChild(label);
    const toggles = buildModToggles();
    side.appendChild(toggles);
    container.appendChild(side);
    return toggles;
}

function buildModToggles() {
    const toggles = document.createElement('div');
    toggles.className = 'mod-toggles';
    for (const mod of KNOWN_MODS) {
        const chip = document.createElement('button');
        chip.type = 'button';
        chip.className = 'mod-chip';
        chip.textContent = mod;
        chip.dataset.mod = mod;
        chip.addEventListener('click', () => chip.classList.toggle('active'));
        toggles.appendChild(chip);
    }
    return toggles;
}

function buildBooleanInput(container, fieldKey) {
    const label = document.createElement('label');
    label.className = 'global-toggle';
    const checkbox = document.createElement('input');
    checkbox.type = 'checkbox';
    const span = document.createElement('span');
    span.textContent = prettyLabel(fieldKey);
    label.append(checkbox, span);
    container.appendChild(label);
    return { type: 'boolean', el: checkbox };
}

function buildStringArrayInput(container, fieldKey) {
    const input = document.createElement('input');
    input.type = 'text';
    input.className = 'key-input keys-input';
    input.placeholder = `${prettyLabel(fieldKey)} (comma-separated)`;
    container.appendChild(input);
    return { type: 'string-array', el: input };
}

function buildScalarInput(container, fieldKey, fieldType) {
    const input = document.createElement('input');
    input.type = fieldType === 'number' ? 'number' : 'text';
    input.className = 'key-input';
    input.placeholder = prettyLabel(fieldKey);
    container.appendChild(input);
    return { type: fieldType, el: input };
}

function collectItem(inputs) {
    const item = {};
    for (const [fieldKey, info] of Object.entries(inputs)) {
        if (info.type === 'mods') { item[fieldKey] = Array.from(info.el.querySelectorAll('.mod-chip.active')).map(b => b.dataset.mod); continue; }
        if (info.type === 'boolean') { item[fieldKey] = info.el.checked; continue; }
        if (info.type === 'string-array') { item[fieldKey] = info.el.value.split(',').map(v => v.trim().toLowerCase()).filter(Boolean); continue; }
        if (info.type === 'number') { item[fieldKey] = parseFloat(info.el.value) || 0; continue; }
        item[fieldKey] = info.el.value.trim();
    }
    return item;
}

function hasItemContent(item) {
    return Object.entries(item).some(([fieldKey, fieldValue]) => {
        if (fieldKey === 'global') return false;
        if (Array.isArray(fieldValue)) return fieldValue.length > 0;
        if (typeof fieldValue === 'string') return fieldValue.length > 0;
        return true;
    });
}

function resetInputs(inputs) {
    for (const info of Object.values(inputs)) {
        if (info.type === 'mods') {
            info.el.querySelectorAll('.mod-chip').forEach(chip => chip.classList.remove('active'));
        } else if (info.type === 'boolean') {
            info.el.checked = false;
        } else {
            info.el.value = '';
        }
    }
}

function handleAdd(inputs, arrayPath, state, rerender) {
    const item = collectItem(inputs);
    if (!hasItemContent(item)) return;
    let values = getVal(state.config, arrayPath);
    if (!values) {
        setVal(state.config, arrayPath, []);
        values = getVal(state.config, arrayPath);
    }
    values.push(item);
    resetInputs(inputs);
    rerender();
}
