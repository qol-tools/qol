import { useEffect, useRef, useState } from 'preact/hooks';

const MAX_BACKOFF_MULTIPLIER = 8;

export function useQueryPoll(pluginId, queryName, intervalMs) {
    const [state, setState] = useState({ data: null, error: null, loading: true });
    const mountedRef = useRef(true);

    useEffect(() => {
        if (!pluginId || !queryName) {
            return undefined;
        }
        mountedRef.current = true;
        let cancelled = false;
        let timer = null;
        let consecutiveFailures = 0;

        const schedule = () => {
            if (cancelled || intervalMs <= 0) return;
            const multiplier = Math.min(
                MAX_BACKOFF_MULTIPLIER,
                Math.pow(2, consecutiveFailures),
            );
            timer = setTimeout(fetchOnce, intervalMs * multiplier);
        };

        const fetchOnce = async () => {
            if (cancelled || !mountedRef.current) {
                return;
            }
            try {
                const response = await fetch(
                    `/api/plugins/${encodeURIComponent(pluginId)}/queries/${encodeURIComponent(queryName)}`,
                );
                if (!response.ok) {
                    const text = await response.text();
                    throw new Error(text || `HTTP ${response.status}`);
                }
                const data = await response.json();
                if (!cancelled && mountedRef.current) {
                    consecutiveFailures = 0;
                    setState({ data, error: null, loading: false });
                }
            } catch (error) {
                if (cancelled || !mountedRef.current) {
                    return;
                }
                consecutiveFailures++;
                const message = error instanceof Error ? error.message : String(error);
                setState(prev => ({ data: prev.data, error: message, loading: false }));
            }
            schedule();
        };

        fetchOnce();

        return () => {
            cancelled = true;
            mountedRef.current = false;
            if (timer !== null) {
                clearTimeout(timer);
            }
        };
    }, [pluginId, queryName, intervalMs]);

    return state;
}
