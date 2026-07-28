import { useState, useCallback } from 'preact/hooks';

let cachedApps = null;

export function useApps() {
    const [apps, setApps] = useState(cachedApps);
    const [loading, setLoading] = useState(false);

    const fetchApps = useCallback(async () => {
        if (cachedApps) { setApps(cachedApps); return; }
        if (loading) return;

        setLoading(true);
        try {
            const res = await fetch('/api/apps');
            cachedApps = res.ok ? await res.json() : [];
        } catch {
            cachedApps = [];
        }
        setApps(cachedApps);
        setLoading(false);
    }, [loading]);

    return { apps, loading, fetchApps };
}

export function appNameFor(bundleId, apps) {
    if (!apps) return null;
    const entry = apps.find(a => a.bundle_id === bundleId);
    return entry ? entry.name : null;
}
