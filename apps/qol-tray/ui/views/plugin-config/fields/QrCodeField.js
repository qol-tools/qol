import { html } from '../../../lib/html.js';
import { useEffect, useRef } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { useQueryPoll } from '../../../hooks/useQueryPoll.js';

const DEFAULT_POLL_MS = 5000;

export function QrCodeField({ field }) {
    const ctx = usePluginConfigContext();
    const queryDef = ctx.runtime?.query?.[field.query];
    const interval = queryDef?.poll_interval_ms || DEFAULT_POLL_MS;
    const { data, error } = useQueryPoll(ctx.pluginId, field.query, interval);
    const canvasRef = useRef(null);

    const rawValue = data && field.value_from ? extractPath(data, field.value_from) : null;
    const url = rawValue == null ? null : String(rawValue);

    useEffect(() => {
        if (!url || !canvasRef.current) {
            return;
        }
        renderQrPlaceholder(canvasRef.current, url);
    }, [url]);

    return html`
        <div class="field-group field-qr-code"
            data-plugin-config-field-id=${field.id}>
            <div class="field-qr-label">${field.label}</div>
            ${url
                ? html`<canvas ref=${canvasRef} class="qr-canvas" width="256" height="256" />`
                : html`<div class="field-qr-placeholder">${field.placeholder || 'Waiting...'}</div>`
            }
            ${error && html`<div class="field-qr-error">${error}</div>`}
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

function renderQrPlaceholder(canvas, url) {
    const ctx = canvas.getContext('2d');
    if (!ctx) {
        return;
    }
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.fillStyle = '#000';
    ctx.font = '12px sans-serif';
    ctx.fillText('QR: ' + url.slice(0, 32), 8, 20);
}
