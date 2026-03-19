import { html } from '../../../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { FieldLabel } from './FieldLabel.js';
import { fieldSelectionClasses } from '../field-map.js';

export function NumberField({ field }) {
    const ctx = usePluginConfigContext();
    const value = ctx.getFieldValue(field);
    const { min, max, step } = field.number;
    const unit = inferUnit(field);
    const resolvedStep = step ?? 1;
    const selected = ctx.selectedFieldId === field.id;
    const index = ctx.fieldIndexById[field.id];
    const inputRef = useRef(null);
    const displayRef = useRef(null);
    const editInitRef = useRef(null);
    const wasEditingRef = useRef(false);
    const [editing, setEditing] = useState(false);

    const apply = useCallback((v) => {
        const clamped = clamp(v, min, max);
        ctx.setFieldValue(field, clamped);
        ctx.save();
    }, [field, ctx, min, max]);

    const onKeyDown = useCallback((e) => {
        if (e.key === 'ArrowUp') { e.preventDefault(); e.stopPropagation(); apply(value + resolvedStep); return; }
        if (e.key === 'ArrowDown') { e.preventDefault(); e.stopPropagation(); apply(value - resolvedStep); return; }
        if (e.key === 'Enter') { e.preventDefault(); e.stopPropagation(); editInitRef.current = null; setEditing(true); return; }
        if (e.key === 'Backspace') {
            e.preventDefault();
            e.stopPropagation();
            editInitRef.current = '';
            setEditing(true);
            return;
        }
        if (/^[0-9.\-]$/.test(e.key)) {
            e.preventDefault();
            e.stopPropagation();
            editInitRef.current = e.key;
            setEditing(true);
        }
    }, [value, resolvedStep, apply]);
    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    const commitEdit = useCallback(() => {
        if (!inputRef.current) return;
        const raw = Number(inputRef.current.value);
        if (!Number.isNaN(raw)) apply(raw);
        setEditing(false);
    }, [apply]);

    const onEditKeyDown = useCallback((e) => {
        if (e.key === 'Enter') { e.preventDefault(); commitEdit(); return; }
        if (e.key === 'Escape') { e.preventDefault(); setEditing(false); e.stopPropagation(); return; }
        if (e.key === 'Tab') return;
        e.stopPropagation();
    }, [commitEdit]);

    const onWheel = useCallback((e) => {
        e.preventDefault();
        apply(value + (e.deltaY < 0 ? resolvedStep : -resolvedStep));
    }, [value, resolvedStep, apply]);

    useEffect(() => {
        if (!editing) {
            if (!wasEditingRef.current) return;
            wasEditingRef.current = false;
            return;
        }
        wasEditingRef.current = true;
        if (!inputRef.current) return;
        inputRef.current.focus();
        const init = editInitRef.current;
        if (init !== null) {
            inputRef.current.value = init;
            inputRef.current.setSelectionRange(init.length, init.length);
            return;
        }
        inputRef.current.select();
    }, [editing]);

    return html`
        <div tabIndex="-1" class="field-group ${fieldSelectionClasses(selected)}"
            data-plugin-config-field-id=${field.id}
            data-plugin-config-index=${index}
            data-selected-surface=""
            data-selected=${selected ? 'true' : 'false'}
            onMouseDown=${onSelect}
            onFocus=${onSelect}>
            <${FieldLabel} text=${field.label} description=${field.description || ''} />
            ${editing
                ? html`<input ref=${inputRef} type="text" class="number-edit" tabIndex="0" data-wedge-root=""
                    value=${formatValue(value)} onBlur=${commitEdit} onKeyDown=${onEditKeyDown} />`
                : html`<div ref=${displayRef} class="number-display" tabIndex="0" data-wedge-root="" onKeyDown=${onKeyDown}
                    onWheel=${onWheel} onClick=${() => { editInitRef.current = null; setEditing(true); }}>
                    <span class="number-value">${formatValue(value)}</span>
                    ${unit && html`<span class="number-unit">${unit}</span>`}
                </div>`
            }
        </div>
    `;
}

function clamp(value, min, max) {
    if (min !== null && value < min) return min;
    if (max !== null && value > max) return max;
    return value;
}

function inferUnit(field) {
    if (field.id.endsWith('_percent')) return '%';
    if (field.id.endsWith('_px') || field.id.endsWith('_pixels')) return 'px';
    return '';
}

function formatValue(v) {
    if (Number.isInteger(v)) return `${v}`;
    return `${parseFloat(v.toFixed(4))}`;
}
