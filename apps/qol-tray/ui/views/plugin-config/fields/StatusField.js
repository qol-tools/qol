import { html } from '../../../lib/html.js';
import { useEffect } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { useQueryPoll } from '../../../hooks/useQueryPoll.js';

const DEFAULT_POLL_MS = 2000;

export function StatusField({ field }) {
    const ctx = usePluginConfigContext();
    const queryDef = ctx.runtime?.query?.[field.query];
    const interval = queryDef?.poll_interval_ms || DEFAULT_POLL_MS;
    const { data, error, loading } = useQueryPoll(ctx.pluginId, field.query, interval);

    const rawValue = data && field.value_from ? extractPath(data, field.value_from) : null;
    const stringValue = rawValue == null ? null : String(rawValue);
    const label = (field.label_map && stringValue && field.label_map[stringValue]) || stringValue;
    const tone = (field.tone_map && stringValue && field.tone_map[stringValue]) || 'neutral';
    const displayText = loading && !data ? 'Loading...' : error ? 'Error' : (label || '—');

    const effectiveTone = (loading && !data) ? 'danger' : (error ? 'danger' : tone);

    useEffect(() => {
        ctx.reportStatusTone?.(field.id, effectiveTone);
    }, [effectiveTone, field.id]);

    return html`
        <div class="field-group field-status"
            data-plugin-config-field-id=${field.id}>
            <div class="status-label">${field.label}</div>
            <div class="status-chip status-chip--${tone}">${displayText}</div>
            ${error && html`<div class="field-status-error">${error}</div>`}
        </div>
    `;
}

function extractPath(obj, path) {
    const parts = path.split('.');
    let current = obj;
    for (const part of parts) {
        if (current == null || typeof current !== 'object') {
            return null;
        }
        current = current[part];
    }
    return current;
}
