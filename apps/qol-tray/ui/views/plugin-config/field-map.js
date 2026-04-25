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
import { SliderField } from './fields/SliderField.js';
import { CustomSelect } from '../../lib/components/CustomSelect.js';
import { ToggleSwitch } from '../../lib/components/ToggleSwitch.js';
import { useSurface } from '../../lib/components/Surface.js';
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
    if (field.kind === 'number' && field.variant === 'slider') {
        return html`<${SliderField} key=${field.id} field=${field} />`;
    }
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

    return html`
        <div ...${fieldSurfaceAttrs(field, ctx, 'toggle-row')}
            onMouseDown=${onSelect} onFocus=${onSelect}>
            <${ToggleSwitch} checked=${checked} onChange=${onChange}
                label=${field.label} description=${field.description || ''} />
        </div>
    `;
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

export function fieldLayoutAttrs(field) {
    const attrs = {
        'data-plugin-config-field-id': field.id,
    };
    if (field.align) attrs['data-field-align'] = field.align;
    if (field.span) attrs['data-field-span'] = String(field.span);
    return attrs;
}

export function fieldSurfaceAttrs(field, ctx, baseClass) {
    const selected = ctx.selectedFieldId === field.id;
    const index = ctx.fieldIndexById[field.id];
    const { attrs } = useSurface({ selected: selected ? true : undefined });
    const result = {
        ...fieldLayoutAttrs(field),
        ...attrs,
        class: selected ? `${baseClass} selected is-selected` : baseClass,
    };
    if (index !== undefined) result['data-plugin-config-index'] = index;
    return result;
}
