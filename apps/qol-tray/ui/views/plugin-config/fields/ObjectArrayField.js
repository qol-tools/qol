import { html } from '../../../lib/html.js';
import { useState, useCallback, useRef } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { FieldLabel } from './FieldLabel.js';
import { fieldSurfaceAttrs } from '../field-map.js';
import { ToggleSwitch } from '../../../components/ToggleSwitch.js';
import { groupFields } from '../../../auto-config/object-array-form.js';
import { declaredFieldsToSchema } from '../../../auto-config/object-array-renderer.js';
import { KNOWN_MODS, prettyLabel, getObjectArraySchema, guessSchemaFromKey } from '../../../auto-config/heuristics.js';

export function ObjectArrayField({ field }) {
    const ctx = usePluginConfigContext();
    const [, setTick] = useState(0);
    const values = ctx.getFieldValue(field) || [];
    const schema = resolveSchema(field, values);

    const remove = useCallback((i) => {
        values.splice(i, 1);
        setTick(t => t + 1);
        ctx.save();
    }, [values, ctx]);

    const add = useCallback((item) => {
        values.push(item);
        setTick(t => t + 1);
        ctx.save();
    }, [values, ctx]);

    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    return html`
        <div ...${fieldSurfaceAttrs(field, ctx, 'field-group')}
            onMouseDown=${onSelect}
            onFocus=${onSelect}>
            <${FieldLabel} text=${field.label} description=${field.description || ''} />
            <div class="object-array-panel">
                <${RuleList} items=${values} remove=${remove} />
                <${AddRuleForm} schema=${schema} onAdd=${add} />
            </div>
        </div>
    `;
}

function resolveSchema(field, values) {
    if (field.item?.fields) return declaredFieldsToSchema(field.item.fields);
    if (values.length > 0) return getObjectArraySchema(values);
    return guessSchemaFromKey(field.config_key || field.id);
}

function RuleList({ items, remove }) {
    if (items.length === 0) return html`<div class="field-empty">No items.</div>`;
    return html`
        <div class="rules-list">
            ${items.map((item, i) => html`
                <div key=${i} class="rule-row" data-wedge-root="" data-selection-tint-root="">
                    <${RuleRowContent} item=${item} />
                    <button type="button" class="btn-remove" onClick=${() => remove(i)}>\u00d7</button>
                </div>
            `)}
        </div>
    `;
}

function RuleRowContent({ item }) {
    const entries = Object.entries(item).filter(([k]) => k !== 'global');
    const fromEntries = entries.filter(([k]) => k.startsWith('from_'));
    const toEntries = entries.filter(([k]) => k.startsWith('to_'));
    const rest = entries.filter(([k]) => !k.startsWith('from_') && !k.startsWith('to_'));
    const hasDirectional = fromEntries.length > 0 || toEntries.length > 0;

    if (!hasDirectional) {
        return html`
            ${entries.map(([k, v]) => html`<${FieldChip} key=${k} fieldKey=${k} value=${v} />`)}
            ${item.global && html`<span class="global-badge">global</span>`}
        `;
    }

    return html`
        <${RuleSide} entries=${fromEntries} />
        ${rest.map(([k, v]) => html`<${FieldChip} key=${k} fieldKey=${k} value=${v} />`)}
        <span class="arrow">\u2192</span>
        <${RuleSide} entries=${toEntries} />
        ${item.global && html`<span class="global-badge">global</span>`}
    `;
}

function RuleSide({ entries }) {
    return html`
        <div class="rule-side">
            ${entries.map(([k, v]) => html`<${FieldChip} key=${k} fieldKey=${k} value=${v} />`)}
        </div>
    `;
}

function FieldChip({ fieldKey, value }) {
    if (Array.isArray(value) && fieldKey.endsWith('_mods')) {
        return html`${value.map(m => html`<span key=${m} class="mod-chip-static">${m}</span>`)}`;
    }
    if (Array.isArray(value)) {
        return html`${value.map(k => html`<span key=${k} class="key-chip">${k}</span>`)}`;
    }
    if (typeof value === 'string' && value) {
        return html`<span class="key-label">${value}</span>`;
    }
    return null;
}

function AddRuleForm({ schema, onAdd }) {
    const formRef = useRef({});
    const [, setTick] = useState(0);
    const schemaEntries = Array.isArray(schema) ? schema : Array.from(schema.entries());
    const { fromFields, toFields, rest, booleans } = groupFields(schemaEntries);

    const handleAdd = useCallback(() => {
        const item = collectFormValues(formRef.current);
        if (!hasContent(item)) return;
        onAdd(item);
        resetForm(formRef.current);
        setTick(t => t + 1);
    }, [onAdd]);

    return html`
        <div class="add-rule-row">
            ${fromFields.length > 0 && html`
                <div class="add-rule-group">
                    ${fromFields.map(([k, t]) => html`<${FormField} key=${k} fieldKey=${k} fieldType=${t} formRef=${formRef} />`)}
                </div>
            `}
            ${fromFields.length > 0 && toFields.length > 0 && html`<span class="arrow">\u2192</span>`}
            ${toFields.length > 0 && html`
                <div class="add-rule-group">
                    ${toFields.map(([k, t]) => html`<${FormField} key=${k} fieldKey=${k} fieldType=${t} formRef=${formRef} />`)}
                </div>
            `}
            ${rest.map(([k, t]) => html`<${FormField} key=${k} fieldKey=${k} fieldType=${t} formRef=${formRef} />`)}
            ${booleans.map(([k]) => html`<${BooleanToggle} key=${k} fieldKey=${k} formRef=${formRef} />`)}
            <button type="button" class="btn btn-ghost btn-sm btn-add"
                data-wedge-root="" onClick=${handleAdd}>+ Add</button>
        </div>
    `;
}

function FormField({ fieldKey, fieldType, formRef }) {
    if (fieldType === 'mods') return html`<${ModToggleGroup} fieldKey=${fieldKey} formRef=${formRef} />`;
    if (fieldType === 'string-array') return html`<${StringArrayInput} fieldKey=${fieldKey} formRef=${formRef} />`;
    return html`<${ScalarInput} fieldKey=${fieldKey} fieldType=${fieldType} formRef=${formRef} />`;
}

function ModToggleGroup({ fieldKey, formRef }) {
    const [active, setActive] = useState(new Set());
    formRef.current[fieldKey] = { type: 'mods', get: () => [...active] };

    const toggle = useCallback((mod) => {
        setActive(prev => {
            const next = new Set(prev);
            if (next.has(mod)) next.delete(mod);
            else next.add(mod);
            return next;
        });
    }, []);

    return html`
        <div class="rule-side">
            <div class="field-label">${prettyLabel(fieldKey)}</div>
            <div class="mod-toggles">
                ${KNOWN_MODS.map(mod => html`
                    <button key=${mod} type="button"
                        class="mod-chip ${active.has(mod) ? 'active' : ''}"
                        data-wedge-root=""
                        onClick=${() => toggle(mod)}>${mod}</button>
                `)}
            </div>
        </div>
    `;
}

function StringArrayInput({ fieldKey, formRef }) {
    const ref = useRef(null);
    formRef.current[fieldKey] = {
        type: 'string-array',
        get: () => ref.current?.value.split(',').map(v => v.trim().toLowerCase()).filter(Boolean) || [],
        reset: () => { if (ref.current) ref.current.value = ''; },
    };
    return html`<input ref=${ref} type="text"
        class="key-input keys-input" data-wedge-root="" data-selection-tint-root=""
        placeholder="${prettyLabel(fieldKey)} (comma-separated)" />`;
}

function ScalarInput({ fieldKey, fieldType, formRef }) {
    const ref = useRef(null);
    formRef.current[fieldKey] = {
        type: fieldType,
        get: () => ref.current?.value.trim() || '',
        reset: () => { if (ref.current) ref.current.value = ''; },
    };
    return html`<input ref=${ref} type=${fieldType === 'number' ? 'number' : 'text'}
        class="key-input" data-wedge-root="" data-selection-tint-root=""
        placeholder=${prettyLabel(fieldKey)} />`;
}

function BooleanToggle({ fieldKey, formRef }) {
    const [checked, setChecked] = useState(false);
    formRef.current[fieldKey] = { type: 'boolean', get: () => checked, reset: () => setChecked(false) };
    return html`<${ToggleSwitch} checked=${checked} onChange=${setChecked} label=${prettyLabel(fieldKey)} />`;
}

function collectFormValues(form) {
    const item = {};
    for (const [key, field] of Object.entries(form)) {
        item[key] = field.get();
    }
    return item;
}

function hasContent(item) {
    return Object.entries(item).some(([k, v]) => {
        if (k === 'global') return false;
        if (Array.isArray(v)) return v.length > 0;
        if (typeof v === 'string') return v.length > 0;
        return true;
    });
}

function resetForm(form) {
    for (const field of Object.values(form)) {
        if (typeof field.reset === 'function') field.reset();
    }
}
