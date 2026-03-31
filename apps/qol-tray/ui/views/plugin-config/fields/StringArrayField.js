import { html } from '../../../lib/html.js';
import { useState, useCallback, useRef } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { FieldLabel } from './FieldLabel.js';
import { fieldSelectionClasses } from '../field-map.js';

export function StringArrayField({ field }) {
    const ctx = usePluginConfigContext();
    const [, setTick] = useState(0);
    const inputRef = useRef(null);
    const values = ctx.getFieldValue(field) || [];
    const showIcon = hasAppIcon(field);
    const selected = ctx.selectedFieldId === field.id;
    const index = ctx.fieldIndexById[field.id];

    const remove = useCallback((index) => {
        values.splice(index, 1);
        setTick(t => t + 1);
        ctx.save();
    }, [values, ctx]);

    const add = useCallback(() => {
        addItem(inputRef, values, setTick, ctx);
    }, [values, ctx]);

    const onKeyDown = useCallback((e) => {
        if (e.key !== 'Enter') return;
        e.preventDefault();
        e.stopPropagation();
        add();
    }, [add]);
    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    return html`
        <div class="field-group ${fieldSelectionClasses(selected)}"
            data-plugin-config-field-id=${field.id}
            data-plugin-config-index=${index}
            data-selected-surface=""
            data-selected=${selected ? 'true' : 'false'}
            onMouseDown=${onSelect}
            onFocus=${onSelect}>
            <${FieldLabel} text=${field.label} description=${field.description || ''} />
            <${StringList} values=${values} showIcon=${showIcon} remove=${remove} />
            <${AddRow} inputRef=${inputRef} placeholder=${field.placeholder} onKeyDown=${onKeyDown} add=${add} />
        </div>
    `;
}

function hasAppIcon(field) {
    return field.id === 'excluded_apps' || field.id.endsWith('_apps') || field.id.endsWith('_bundles');
}

function addItem(inputRef, values, setTick, ctx) {
    const v = inputRef.current?.value?.trim();
    if (!v || values.includes(v)) return;
    values.push(v);
    inputRef.current.value = '';
    setTick(t => t + 1);
    ctx.save();
}

function StringList({ values, showIcon, remove }) {
    return html`
        <div class="string-list">
            ${values.length === 0 && html`<div class="field-empty">No items.</div>`}
            ${values.map((v, i) => html`
                <div key=${`${v}-${i}`} class="string-item" data-wedge-root="" data-selection-tint-root="">
                    ${showIcon && html`<img class="app-icon" src=${`/api/icon/${encodeURIComponent(v)}`}
                        width="20" height="20" onError=${hideIcon} />`}
                    <span>${v}</span>
                    <button type="button" class="btn-remove"
                        onClick=${() => remove(i)}>\u00d7</button>
                </div>
            `)}
        </div>
    `;
}

function AddRow({ inputRef, placeholder, onKeyDown, add }) {
    return html`
        <div class="add-row">
            <input ref=${inputRef} type="text" class="text-input" data-wedge-root="" data-selection-tint-root=""
                placeholder=${placeholder || 'Add item...'} onKeyDown=${onKeyDown} />
            <button type="button" class="btn btn-ghost btn-sm btn-add" data-wedge-root=""
                onClick=${add}>+ Add</button>
        </div>
    `;
}

function hideIcon(e) { e.target.style.display = 'none'; }
