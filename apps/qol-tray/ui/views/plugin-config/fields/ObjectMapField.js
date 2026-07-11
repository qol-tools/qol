import { html } from '../../../lib/html.js';
import { useState, useCallback, useRef } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { FieldLabel } from './FieldLabel.js';
import { fieldSurfaceAttrs } from '../field-map.js';
import { prettyLabel } from '../../../lib/qol-config.js';

export function ObjectMapField({ field }) {
    const ctx = usePluginConfigContext();
    const [, setTick] = useState(0);
    const values = ctx.getFieldValue(field) || {};

    const remove = useCallback((key) => {
        const next = { ...values };
        delete next[key];
        ctx.setFieldValue(field, next);
        setTick(t => t + 1);
        ctx.save();
    }, [field, values, ctx]);

    const add = useCallback((key, entry) => {
        const next = { ...values, [key]: entry };
        ctx.setFieldValue(field, next);
        setTick(t => t + 1);
        ctx.save();
    }, [field, values, ctx]);

    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    return html`
        <div ...${fieldSurfaceAttrs(field, ctx, 'field-group')}
            onMouseDown=${onSelect}
            onFocus=${onSelect}>
            <${FieldLabel} text=${field.label} description=${field.description || ''} />
            <div class="object-array-panel">
                <${EntryList} field=${field} entries=${Object.entries(values)} remove=${remove} />
                <${AddEntryForm} field=${field} onAdd=${add} />
            </div>
        </div>
    `;
}

function EntryList({ field, entries, remove }) {
    if (entries.length === 0) return html`<div class="field-empty">No items.</div>`;
    return html`
        <div class="rules-list">
            ${entries.map(([key, value]) => html`
                <div key=${key} class="rule-row object-map-row" data-wedge-root="" data-selection-tint-root="">
                    <span class="key-label">${field.key_label || 'Key'}: ${key}</span>
                    ${Object.entries(value || {}).map(([name, entryValue]) => html`
                        <span key=${name} class="key-label">${prettyLabel(name)}: ${formatValue(entryValue)}</span>
                    `)}
                    <button type="button" class="btn-remove" onClick=${() => remove(key)}>\u00d7</button>
                </div>
            `)}
        </div>
    `;
}

function AddEntryForm({ field, onAdd }) {
    const keyRef = useRef(null);
    const fieldRefs = useRef({});
    const [, setTick] = useState(0);
    const entryFields = Object.entries(field.entry_fields || {});

    const handleAdd = useCallback(() => {
        const key = keyRef.current?.value.trim();
        if (!key) return;
        const entry = collectEntry(fieldRefs.current);
        onAdd(key, entry);
        resetForm(keyRef, fieldRefs.current);
        setTick(t => t + 1);
    }, [onAdd]);

    const onKeyDown = useCallback((event) => {
        if (event.key !== 'Enter') return;
        event.preventDefault();
        event.stopPropagation();
        handleAdd();
    }, [handleAdd]);

    return html`
        <div class="add-rule-row object-map-add-row">
            <input ref=${keyRef} type="text" class="key-input" data-wedge-root="" data-selection-tint-root=""
                placeholder=${field.key_label || 'Key'} onKeyDown=${onKeyDown} />
            ${entryFields.map(([name, kind]) => html`
                <${EntryInput} key=${name} name=${name} kind=${kind} refs=${fieldRefs} onKeyDown=${onKeyDown} />
            `)}
            <button type="button" class="btn btn-ghost btn-sm btn-add"
                data-wedge-root="" onClick=${handleAdd}>+ Add</button>
        </div>
    `;
}

function EntryInput({ name, kind, refs, onKeyDown }) {
    const ref = useRef(null);
    refs.current[name] = { kind, ref };
    return html`<input ref=${ref} type=${kind === 'number' ? 'number' : 'text'}
        class=${kind === 'string_array' ? 'key-input keys-input' : 'key-input'}
        data-wedge-root="" data-selection-tint-root=""
        placeholder=${prettyLabel(name)} onKeyDown=${onKeyDown} />`;
}

function collectEntry(fields) {
    const entry = {};
    for (const [name, field] of Object.entries(fields)) {
        const value = readEntryValue(field);
        if (value === null) continue;
        entry[name] = value;
    }
    return entry;
}

function readEntryValue(field) {
    const raw = field.ref.current?.value.trim() || '';
    if (!raw) return field.kind === 'string_array' ? [] : null;
    if (field.kind === 'number') return Number(raw);
    if (field.kind === 'boolean') return raw === 'true';
    if (field.kind === 'string_array') {
        return raw.split(',').map(value => value.trim()).filter(Boolean);
    }
    return raw;
}

function resetForm(keyRef, fields) {
    if (keyRef.current) keyRef.current.value = '';
    for (const field of Object.values(fields)) {
        if (field.ref.current) field.ref.current.value = '';
    }
}

function formatValue(value) {
    if (Array.isArray(value)) return value.join(', ');
    return `${value}`;
}
