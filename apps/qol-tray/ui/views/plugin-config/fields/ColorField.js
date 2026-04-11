import { html } from '../../../lib/html.js';
import { useCallback, useEffect, useState } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { fieldSelectionClasses } from '../field-map.js';

export function ColorField({ field }) {
    const ctx = usePluginConfigContext();
    const selected = ctx.selectedFieldId === field.id;
    const index = ctx.fieldIndexById[field.id];
    const stored = ctx.getFieldValue(field);
    const defaultValue = typeof field.default === 'string' ? field.default : '#000000';
    const initial = typeof stored === 'string' ? stored : defaultValue;
    const [local, setLocal] = useState(initial);

    useEffect(() => {
        setLocal(initial);
    }, [initial]);

    const commit = useCallback((nextValue) => {
        if (!isValidHex(nextValue, field.alpha)) {
            return;
        }
        ctx.setFieldValue(field, nextValue);
        ctx.save();
    }, [ctx, field]);

    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    const onPickerInput = useCallback((event) => {
        const next = event.target.value;
        setLocal(next);
        commit(next);
    }, [commit]);

    const onHexInput = useCallback((event) => {
        const next = event.target.value;
        setLocal(next);
        if (isValidHex(next, field.alpha)) {
            commit(next);
        }
    }, [commit, field.alpha]);

    return html`
        <div class="field-group field-color ${fieldSelectionClasses(selected)}"
            data-plugin-config-field-id=${field.id}
            data-plugin-config-index=${index}
            data-selected-surface="" tabIndex="-1"
            data-selected=${selected ? 'true' : 'false'}
            onMouseDown=${onSelect}
            onFocus=${onSelect}>
            <label class="field-color-label">${field.label}</label>
            <div class="field-color-row">
                <input type="color" class="field-color-picker"
                       value=${normalizeForPicker(local)}
                       onInput=${onPickerInput} />
                <input type="text" class="field-color-hex text-input"
                       value=${local}
                       placeholder="#RRGGBB"
                       onInput=${onHexInput} />
            </div>
        </div>
    `;
}

function isValidHex(value, allowAlpha) {
    if (typeof value !== 'string') {
        return false;
    }
    const pattern = allowAlpha ? /^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$/ : /^#[0-9a-fA-F]{6}$/;
    return pattern.test(value);
}

function normalizeForPicker(value) {
    if (typeof value !== 'string') {
        return '#000000';
    }
    if (value.length === 9) {
        return value.slice(0, 7);
    }
    return value;
}
