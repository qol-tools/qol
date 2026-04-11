import { html } from '../../../lib/html.js';
import { useCallback } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { useQueryPoll } from '../../../hooks/useQueryPoll.js';
import { fieldSurfaceAttrs } from '../field-map.js';

const DEFAULT_POLL_MS = 2000;

export function ListField({ field }) {
    const ctx = usePluginConfigContext();
    const queryDef = ctx.runtime?.query?.[field.query];
    const interval = queryDef?.poll_interval_ms || DEFAULT_POLL_MS;
    const { data, error, loading } = useQueryPoll(ctx.pluginId, field.query, interval);

    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    const rows = rowsFrom(data);

    return html`
        <div ...${fieldSurfaceAttrs(field, ctx, 'field-group field-list')}
            onMouseDown=${onSelect}
            onFocus=${onSelect}>
            <div class="field-list-label">${field.label}</div>
            ${renderBody(field, rows, loading, error)}
        </div>
    `;
}

function renderBody(field, rows, loading, error) {
    if (error) {
        return html`<div class="field-list-error">${error}</div>`;
    }
    if (loading && rows.length === 0) {
        return html`<div class="field-list-loading">Loading...</div>`;
    }
    if (rows.length === 0) {
        return html`<div class="field-list-empty">${field.empty_message || 'No items.'}</div>`;
    }
    return rows.map((row, i) => html`
        <div class="list-row" key=${i}>
            <div class="list-row-label">${interpolate(field.row_label, row)}</div>
            ${field.row_subtitle && html`
                <div class="list-row-subtitle">${interpolate(field.row_subtitle, row)}</div>
            `}
        </div>
    `);
}

function rowsFrom(data) {
    if (Array.isArray(data)) {
        return data;
    }
    if (data && Array.isArray(data.items)) {
        return data.items;
    }
    return [];
}

function interpolate(template, row) {
    if (!template || !row) {
        return '';
    }
    return template.replace(/\{(\w+)\}/g, (_, key) => {
        const value = row[key];
        return value == null ? '' : String(value);
    });
}
