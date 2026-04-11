import { html } from '../../lib/html.js';
import { useCallback } from 'preact/hooks';
import { usePluginConfigContext } from './context.js';
import { NumberField } from './fields/NumberField.js';
import { StringArrayField } from './fields/StringArrayField.js';
import { ObjectArrayField } from './fields/ObjectArrayField.js';
import { ColorField } from './fields/ColorField.js';
import { ActionField } from './fields/ActionField.js';
import { ListField } from './fields/ListField.js';
import { StatusField } from './fields/StatusField.js';
import { QrCodeField } from './fields/QrCodeField.js';
import { CustomSelect } from '../../components/CustomSelect.js';
import { FieldLabel } from './fields/FieldLabel.js';

const FIELD_MAP = {
    boolean: BooleanField,
    string: StringField,
    select: SelectField,
    number: NumberField,
    string_array: StringArrayField,
    object_array: ObjectArrayField,
    color: ColorField,
    action: ActionField,
    list: ListField,
    status: StatusField,
    qr_code: QrCodeField,
};

export function renderField(field) {
    const Component = FIELD_MAP[field.kind] || StringField;
    return html`<${Component} key=${field.id} field=${field} />`;
}

function BooleanField({ field }) {
    const ctx = usePluginConfigContext();
    const checked = Boolean(ctx.getFieldValue(field));
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
        fieldAttrs=${fieldSurfaceAttrs(field, ctx, 'toggle-row')}
        onSelect=${onSelect} />`;
}

function StringField({ field }) {
    const ctx = usePluginConfigContext();
    const onInput = useCallback((event) => {
        ctx.setFieldValue(field, event.target.value);
        ctx.save();
    }, [field, ctx]);
    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    return html`
        <div ...${fieldSurfaceAttrs(field, ctx, 'field-group')}
            onMouseDown=${onSelect}
            onFocus=${onSelect}>
            <${FieldLabel} text=${field.label} description=${field.description || ''} />
            <input type="text" class="text-input" data-wedge-root=""
                value=${ctx.getFieldValue(field) || ''}
                placeholder=${field.placeholder || ''}
                onInput=${onInput} />
        </div>
    `;
}

function SelectField({ field }) {
    const ctx = usePluginConfigContext();
    const value = ctx.getFieldValue(field);
    const onChange = useCallback((option) => {
        ctx.setFieldValue(field, option);
        ctx.bumpRender();
        ctx.save();
    }, [field, ctx]);
    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    return html`
        <div ...${fieldSurfaceAttrs(field, ctx, 'field-group')}
            onMouseDown=${onSelect}
            onFocus=${onSelect}>
            <${FieldLabel} text=${field.label} description=${field.description || ''} />
            <${CustomSelect} value=${value} options=${field.options}
                labels=${field.option_labels} onChange=${onChange} />
        </div>
    `;
}

function Toggle({ checked, onChange, label, description, fieldAttrs, onSelect }) {
    const toggle = useCallback(() => onChange(!checked), [checked, onChange]);
    const onKeyDown = useCallback((event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        event.stopPropagation();
        toggle();
    }, [toggle]);

    return html`
        <div ...${fieldAttrs}
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

export function fieldSurfaceAttrs(field, ctx, baseClass) {
    const selected = ctx.selectedFieldId === field.id;
    const index = ctx.fieldIndexById[field.id];
    const attrs = {
        class: selected ? `${baseClass} selected is-selected` : baseClass,
        'data-plugin-config-field-id': field.id,
        'data-selected-surface': '',
        tabIndex: -1,
    };
    if (index !== undefined) attrs['data-plugin-config-index'] = index;
    if (selected) attrs['data-selected'] = 'true';
    return attrs;
}
