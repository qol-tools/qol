import { useEffect, useState } from 'preact/hooks';
import { installSurfaceAudit } from '../lib/surface-audit.js';
import { getTrace, clearTrace } from '../lib/debug.js';

function bootDev() {
    try { return window.__QOL_BOOT__?.dev === true; } catch { return false; }
}

export function useAppBootstrap() {
    const [devEnabled, setDevEnabled] = useState(bootDev);
    const [appVersion, setAppVersion] = useState(null);

    useEffect(() => {
        installSurfaceAudit();
        window.qolTrace = (filter) => { const rows = getTrace(filter); console.table(rows); return rows; };
        window.qolTraceClear = () => clearTrace();
        (async () => {
            if (!window.__QOL_BOOT__) {
                try {
                    const res = await fetch('/api/dev/enabled');
                    if (res.ok) setDevEnabled(await res.json());
                } catch {}
            }

            try {
                const res = await fetch('/api/version');
                if (res.ok) {
                    setAppVersion(await res.text());
                }
            } catch {}
        })();
    }, []);

    return { devEnabled, appVersion };
}
