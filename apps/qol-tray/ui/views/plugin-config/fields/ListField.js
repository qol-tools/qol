import { html } from '../../../lib/html.js';
import { useCallback } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { useQueryPoll } from '../../../lib/hooks/useQueryPoll.js';
import { useDispatchAction } from '../../../lib/hooks/useDispatchAction.js';
import { fieldSurfaceAttrs } from '../field-map.js';
import { visibleRowAction, firstActionableRow } from './row-action.js';

const DEFAULT_POLL_MS = 2000;

export function ListField({ field }) {
    const ctx = usePluginConfigContext();
    const queryDef = ctx.runtime?.query?.[field.query];
    const interval = queryDef?.poll_interval_ms || DEFAULT_POLL_MS;
    const { data, error, loading } = useQueryPoll(ctx.pluginId, field.query, interval);
    const rowDispatch = useDispatchAction(ctx.pluginId, field.row_action?.action);

    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    const rows = rowsFrom(data);

    const runRowAction = useCallback((row) => {
        if (rowDispatch.pending || !visibleRowAction(field.row_action, row)) return;
        rowDispatch.dispatch().catch(() => {});
    }, [rowDispatch, field.row_action]);

    const onKeyDown = useCallback((event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        const row = firstActionableRow(field.row_action, rows);
        if (!row) return;
        event.preventDefault();
        event.stopPropagation();
        runRowAction(row);
    }, [field.row_action, rows, runRowAction]);

    if (ctx.isRuntimeDisabled) {
        return html`
            <div ...${fieldSurfaceAttrs(field, ctx, 'field-group field-list field-gated')}
                onMouseDown=${onSelect}
                onFocus=${onSelect}>
                <div class="field-list-label">${field.label}</div>
                <div class="field-list-empty">${field.empty_message || 'No items.'}</div>
            </div>
        `;
    }

    return html`
        <div ...${fieldSurfaceAttrs(field, ctx, 'field-group field-list')}
            onMouseDown=${onSelect}
            onFocus=${onSelect}
            onKeyDown=${onKeyDown}>
            <div class="field-list-label">${field.label}</div>
            ${renderBody(field, rows, loading, error, runRowAction, rowDispatch.pending)}
            ${rowDispatch.error && html`<div class="field-action-error">${rowDispatch.error}</div>`}
        </div>
    `;
}

function renderBody(field, rows, loading, error, runRowAction, actionPending) {
    if (error) {
        return html`<div class="field-list-error">${error}</div>`;
    }
    if (loading && rows.length === 0) {
        return html`<div class="field-list-loading">Loading...</div>`;
    }
    if (rows.length === 0) {
        return html`<div class="field-list-empty">${field.empty_message || 'No items.'}</div>`;
    }
    return rows.map((row, i) => renderRow(field, row, i, runRowAction, actionPending));
}

function renderRow(field, row, key, runRowAction, actionPending) {
    const action = visibleRowAction(field.row_action, row);
    return html`
        <div class="list-row" key=${key}>
            <div class="list-row-main">
                <div class="list-row-label">${interpolate(field.row_label, row)}</div>
                ${field.row_subtitle && html`
                    <div class="list-row-subtitle">${interpolate(field.row_subtitle, row)}</div>
                `}
            </div>
            ${action && html`
                <button class="btn btn-primary list-row-action"
                    disabled=${actionPending}
                    onClick=${(event) => { event.stopPropagation(); runRowAction(row); }}>
                    ${action.label}
                <//>
            `}
        </div>
    `;
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
