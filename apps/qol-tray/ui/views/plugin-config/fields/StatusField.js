import { html } from '../../../lib/html.js';
import { useEffect, useRef, useState } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { useQueryPoll } from '../../../lib/hooks/useQueryPoll.js';
import { fieldLayoutAttrs } from '../field-map.js';

const DEFAULT_POLL_MS = 2000;
const FAILURE_THRESHOLD = 2;

export function StatusField({ field }) {
    const ctx = usePluginConfigContext();
    const queryDef = ctx.runtime?.query?.[field.query];
    const interval = queryDef?.poll_interval_ms || DEFAULT_POLL_MS;
    const { data, error, loading } = useQueryPoll(ctx.pluginId, field.query, interval);
    const [effectiveTone, setEffectiveTone] = useState('neutral');
    const failCountRef = useRef(0);

    const rawValue = data && field.value_from ? extractPath(data, field.value_from) : null;
    const stringValue = rawValue == null ? null : String(rawValue);
    const label = (field.label_map && stringValue && field.label_map[stringValue]) || stringValue;
    const tone = (field.tone_map && stringValue && field.tone_map[stringValue]) || 'neutral';
    const displayText = loading && !data ? 'Loading...' : error ? 'Error' : (label || '—');

    useEffect(() => {
        if (loading && !data) return;
        if (error) {
            failCountRef.current++;
            if (failCountRef.current >= FAILURE_THRESHOLD) setEffectiveTone('danger');
        } else {
            failCountRef.current = 0;
            setEffectiveTone(tone);
        }
    }, [error, data, loading, tone]);

    useEffect(() => {
        ctx.reportStatusTone?.(field.id, effectiveTone);
    }, [effectiveTone, field.id]);

    return html`
        <div class="field-group field-status" ...${fieldLayoutAttrs(field)}>
            <div class="status-label">${field.label}</div>
            <div class="status-chip status-chip--${effectiveTone}">${displayText}</div>
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
