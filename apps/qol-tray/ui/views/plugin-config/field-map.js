import { html } from '../../lib/html.js';
import { useCallback } from 'preact/hooks';
import { usePluginConfigContext } from './context.js';
import { NumberField } from './fields/NumberField.js';
import { StringArrayField } from './fields/StringArrayField.js';
import { CustomSelect } from './fields/CustomSelect.js';
import { FieldLabel } from './fields/FieldLabel.js';

const FIELD_MAP = {
    boolean: BooleanField,
    string: StringField,
    select: SelectField,
    number: NumberField,
    string_array: StringArrayField,
};

export function renderField(field) {
    const Component = FIELD_MAP[field.kind] || StringField;
    return html`<${Component} key=${field.id} field=${field} />`;
}

function BooleanField({ field }) {
    const ctx = usePluginConfigContext();
    const checked = Boolean(ctx.getFieldValue(field));
    const selected = ctx.selectedFieldId === field.id;
    const index = ctx.fieldIndexById[field.id];
    const onChange = useCallback((value) => {
        ctx.setFieldValue(field, value);
        ctx.bumpRender();
        ctx.save();
    }, [field, ctx]);
    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    return html`<${Toggle} checked=${checked} onChange=${onChange}
        label=${field.label} description=${field.description || ''}
        selected=${selected} index=${index} onSelect=${onSelect} fieldId=${field.id} />`;
}

function StringField({ field }) {
    const ctx = usePluginConfigContext();
    const selected = ctx.selectedFieldId === field.id;
    const index = ctx.fieldIndexById[field.id];
    const onInput = useCallback((event) => {
        ctx.setFieldValue(field, event.target.value);
        ctx.save();
    }, [field, ctx]);
    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    return html`
        <div class="field-group selection-wedge-pseudo-host ${fieldSelectionClasses(selected)}"
            data-plugin-config-field-id=${field.id}
            data-plugin-config-index=${index}
            onMouseDown=${onSelect}
            onFocus=${onSelect}>
            <${FieldLabel} text=${field.label} description=${field.description || ''} />
            <input type="text" class="text-input selection-wedge-pseudo-host" data-wedge-root=""
                value=${ctx.getFieldValue(field) || ''}
                placeholder=${field.placeholder || ''}
                onInput=${onInput} />
        </div>
    `;
}

function SelectField({ field }) {
    const ctx = usePluginConfigContext();
    const value = ctx.getFieldValue(field);
    const selected = ctx.selectedFieldId === field.id;
    const index = ctx.fieldIndexById[field.id];
    const onChange = useCallback((option) => {
        ctx.setFieldValue(field, option);
        ctx.bumpRender();
        ctx.save();
    }, [field, ctx]);
    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    return html`
        <div class="field-group selection-wedge-pseudo-host ${fieldSelectionClasses(selected)}"
            data-plugin-config-field-id=${field.id}
            data-plugin-config-index=${index}
            onMouseDown=${onSelect}
            onFocus=${onSelect}>
            <${FieldLabel} text=${field.label} description=${field.description || ''} />
            <${CustomSelect} value=${value} options=${field.options}
                labels=${field.option_labels} onChange=${onChange} />
        </div>
    `;
}

function Toggle({ checked, onChange, label, description, selected, index, onSelect, fieldId }) {
    const toggle = useCallback(() => onChange(!checked), [checked, onChange]);
    const onKeyDown = useCallback((event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        event.stopPropagation();
        toggle();
    }, [toggle]);

    return html`
        <div class="toggle-row selection-wedge-pseudo-host ${fieldSelectionClasses(selected)}"
            data-plugin-config-field-id=${fieldId}
            data-plugin-config-index=${index}
            onClick=${toggle}
            onMouseDown=${onSelect}
            onFocus=${onSelect}>
            <div class="toggle-track ${checked ? 'on' : ''}" tabIndex="0" role="switch"
                aria-checked=${checked} onKeyDown=${onKeyDown}>
                <div class="toggle-thumb" />
            </div>
            <div class="toggle-label-group">
                <strong>${label}</strong>
                ${description && html`<div class="toggle-help">${description}</div>`}
            </div>
        </div>
    `;
}

function fieldSelectionClasses(selected) {
    if (!selected) return '';
    return 'selected is-selected selection-wedge-visible';
}
