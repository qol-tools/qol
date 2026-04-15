import QRCode from 'https://esm.sh/qrcode@1.5.4';
import { html } from '../../../lib/html.js';
import { useEffect, useRef } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { useQueryPoll } from '../../../lib/hooks/useQueryPoll.js';
import { fieldLayoutAttrs } from '../field-map.js';

const DEFAULT_POLL_MS = 5000;
const QR_SIZE = 256;

export function QrCodeField({ field }) {
    const ctx = usePluginConfigContext();
    const queryDef = ctx.runtime?.query?.[field.query];
    const interval = queryDef?.poll_interval_ms || DEFAULT_POLL_MS;
    const { data, error } = useQueryPoll(ctx.pluginId, field.query, interval);
    const canvasRef = useRef(null);

    const rawValue = data && field.value_from ? extractPath(data, field.value_from) : null;
    const url = rawValue == null ? null : String(rawValue);

    useEffect(() => {
        if (!url || !canvasRef.current) return;
        QRCode.toCanvas(canvasRef.current, url, {
            width: QR_SIZE,
            margin: 2,
            color: { dark: '#000000', light: '#ffffff' },
        }).catch(() => {});
    }, [url]);

    return html`
        <div class="field-group field-qr-code" ...${fieldLayoutAttrs(field)}>
            <div class="field-qr-label">${field.label}</div>
            ${url
                ? html`<canvas ref=${canvasRef} class="qr-canvas" width=${QR_SIZE} height=${QR_SIZE} />`
                : html`<div class="field-qr-placeholder">${field.placeholder || 'Waiting...'}</div>`
            }
            ${url && html`<a class="field-qr-link" href=${url} target="_blank" rel="noopener">${url}</a>`}
            ${error && html`<div class="field-qr-error">${error}</div>`}
        </div>
    `;
}

function extractPath(obj, path) {
    const parts = path.split('.');
    let current = obj;
    for (const part of parts) {
        if (current == null || typeof current !== 'object') return null;
        current = current[part];
    }
    return current;
}
