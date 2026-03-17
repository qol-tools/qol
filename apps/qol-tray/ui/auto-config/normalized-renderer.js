import { createTextFieldLabel } from './field-label.js';
import { prettyLabel } from './heuristics.js';
import { collectVariantGroups, isFieldVisible, optionLabel } from './display-rules.js';
import { configFromForm, getDisplaySections } from './form-model.js';
import { renderNumberField } from './numeric-control.js';
import { renderEmbeddedObjectArray } from './object-array-renderer.js';
import { renderEmbeddedObjectMap } from './object-map-renderer.js';
import { getFieldValue, getFieldValueById, setFieldValue } from './normalized-config.js';
import { sectionActions, createActionBadges } from './action-metadata.js';
import { appendVariantSelectors, renderVariantFields } from './variant-renderer.js';

export { configFromForm, getDisplaySections };

let _iconUrlFor = (value) => `/api/icon/${encodeURIComponent(value)}`;

export function setIconUrl(fn) { _iconUrlFor = fn; }

export function renderSectionDetail(container, sectionData, form, state) {
    container.appendChild(createDetailHeader(sectionData, form));
    const fieldsContainer = document.createElement('div');
    fieldsContainer.className = 'config-section';
    renderSectionFields(fieldsContainer, sectionData.fields, state);
    container.appendChild(fieldsContainer);
}

function createDetailHeader(sectionData, form) {
    const header = document.createElement('header');
    header.className = 'config-detail-header';
    const h2 = document.createElement('h2');
    h2.textContent = sectionData.label || prettyLabel(sectionData.id);
    header.appendChild(h2);
    if (sectionData.description) {
        const p = document.createElement('p');
        p.className = 'section-copy';
        p.textContent = sectionData.description;
        header.appendChild(p);
    }
    if (hasDistinctActions(form.sections)) {
        const actions = sectionActions(sectionData);
        if (actions.length > 0) header.appendChild(createActionBadges(actions));
    }
    return header;
}

function hasDistinctActions(sections) {
    const actionSets = sections
        .filter(section => section.fields.length > 0)
        .map(section => (section.actions || []).slice().sort().join(','));
    const unique = new Set(actionSets);
    return unique.size > 1;
}

function renderSectionFields(container, fields, state) {
    const variantGroups = collectVariantGroups(fields);
    const context = {
        fields,
        state,
        groups: variantGroups,
        renderField: field => renderField(field, state),
        isFieldVisible: field => isFieldVisible(field, fieldId => getFieldValueById(state, fieldId)),
    };
    appendVariantSelectors(context);
    renderVariantFields(container, context);
}

function renderField(field, state) {
    if (field.kind === 'boolean') {
        return renderBooleanField(field, state);
    }
    if (field.kind === 'number') {
        return renderNumberField(field, state, createFieldGroup);
    }
    if (field.kind === 'select') {
        return renderSelectField(field, state);
    }
    if (field.kind === 'string_array') {
        return renderStringArrayField(field, state);
    }
    if (field.kind === 'object_array') {
        return renderEmbeddedObjectArray(field, state);
    }
    if (field.kind === 'object_map') {
        return renderEmbeddedObjectMap(field, state);
    }
    return renderStringField(field, state);
}

function renderBooleanField(field, state) {
    const row = document.createElement('div');
    row.className = 'field-group toggle-row';

    const label = document.createElement('label');
    const checkbox = document.createElement('input');
    checkbox.type = 'checkbox';
    checkbox.checked = Boolean(getFieldValue(state, field));
    checkbox.addEventListener('change', () => {
        setFieldValue(state, field, checkbox.checked);
        onFieldChange(state);
    });

    const text = document.createElement('strong');
    text.textContent = field.label;

    label.append(checkbox, text);
    row.appendChild(label);
    if (field.description) {
        row.appendChild(createInlineHelp(field.description));
    }
    return row;
}

function renderStringField(field, state) {
    const group = createFieldGroup(field);
    const input = document.createElement('input');
    input.type = 'text';
    input.className = 'text-input';
    input.value = getFieldValue(state, field) || '';
    input.placeholder = field.placeholder || '';
    input.addEventListener('input', () => {
        setFieldValue(state, field, input.value);
        state.save();
    });
    group.appendChild(input);
    return group;
}

function renderSelectField(field, state) {
    const group = createFieldGroup(field);
    const select = document.createElement('select');
    select.className = 'field-select';
    for (const option of field.options) {
        const element = document.createElement('option');
        element.value = option;
        element.textContent = optionLabel(field, option);
        if (option === getFieldValue(state, field)) {
            element.selected = true;
        }
        select.appendChild(element);
    }
    select.addEventListener('change', () => {
        setFieldValue(state, field, select.value);
        onFieldChange(state);
    });
    group.appendChild(select);
    return group;
}

function renderStringArrayField(field, state) {
    const group = createFieldGroup(field);
    const list = document.createElement('div');
    list.className = 'string-list';
    const render = () => renderStringArrayItems(list, field, state, render);
    group.appendChild(list);
    group.appendChild(createStringArrayAddRow(field, state, render));
    render();
    return group;
}

function renderStringArrayItems(list, field, state, render) {
    const values = getFieldValue(state, field) || [];
    list.replaceChildren();
    if (values.length === 0) {
        const emptyState = document.createElement('div');
        emptyState.className = 'empty-state';
        emptyState.textContent = 'No items.';
        list.appendChild(emptyState);
        return;
    }
    values.forEach((value, index) => {
        list.appendChild(renderStringArrayItem(field, value, index, state, render));
    });
}

function renderStringArrayItem(field, value, index, state, render) {
    const row = document.createElement('div');
    row.className = 'string-item';
    if (showsAppIcon(field.id)) {
        row.appendChild(createAppIcon(value));
    }
    const text = document.createElement('span');
    text.textContent = value;
    row.appendChild(text);
    const remove = document.createElement('button');
    remove.type = 'button';
    remove.className = 'btn-remove';
    remove.textContent = '×';
    remove.addEventListener('click', () => {
        getFieldValue(state, field).splice(index, 1);
        render();
        state.save();
    });
    row.appendChild(remove);
    return row;
}

function createStringArrayAddRow(field, state, render) {
    const row = document.createElement('div');
    row.className = 'add-row';
    const input = document.createElement('input');
    input.type = 'text';
    input.className = 'text-input';
    input.placeholder = field.placeholder || 'Add item...';
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'btn btn-ghost btn-sm btn-add';
    button.textContent = '+ Add';
    button.addEventListener('click', () => addStringArrayItem(input, field, state, render));
    input.addEventListener('keydown', event => {
        if (event.key !== 'Enter') {
            return;
        }
        event.preventDefault();
        addStringArrayItem(input, field, state, render);
    });
    row.append(input, button);
    return row;
}

function addStringArrayItem(input, field, state, render) {
    const nextValue = input.value.trim();
    if (!nextValue) {
        return;
    }
    const values = getFieldValue(state, field);
    if (values.includes(nextValue)) {
        return;
    }
    values.push(nextValue);
    input.value = '';
    render();
    state.save();
}

function showsAppIcon(fieldId) {
    return fieldId === 'excluded_apps' || fieldId.endsWith('_apps') || fieldId.endsWith('_bundles');
}

function createAppIcon(value) {
    const icon = document.createElement('img');
    icon.className = 'app-icon';
    icon.src = _iconUrlFor(value);
    icon.width = 20;
    icon.height = 20;
    icon.onerror = () => {
        icon.style.display = 'none';
    };
    return icon;
}

function createFieldGroup(field) {
    const group = document.createElement('div');
    group.className = 'field-group';
    group.appendChild(createTextFieldLabel(field.label, field.description || ''));
    return group;
}

function createInlineHelp(description) {
    const help = document.createElement('div');
    help.className = 'toggle-help';
    help.textContent = description;
    return help;
}

function onFieldChange(state) {
    if (typeof state.render === 'function') {
        state.render();
    }
    if (typeof state.save === 'function') {
        state.save();
    }
}
