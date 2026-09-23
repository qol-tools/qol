const UI_TRACE_URL = '/api/trace/ui';

export function traceWorld(event, fields = {}) {
    const payload = JSON.stringify({
        event,
        fields: compactFields(fields),
    });

    try {
        if (typeof navigator !== 'undefined' && navigator.sendBeacon && typeof Blob !== 'undefined') {
            const blob = new Blob([payload], { type: 'application/json' });
            navigator.sendBeacon(UI_TRACE_URL, blob);
            return;
        }
        if (typeof fetch !== 'function') return;
        fetch(UI_TRACE_URL, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: payload,
            keepalive: true,
        }).catch(() => {});
    } catch {}
}

function compactFields(fields) {
    const out = {};
    for (const [key, value] of Object.entries(fields || {})) {
        if (value === undefined || value === null) continue;
        out[key] = String(value).slice(0, 160);
    }
    return out;
}
