import { useCallback, useEffect, useRef, useState } from 'preact/hooks';

const MAX_BACKOFF_MULTIPLIER = 8;

export function useQueryPoll(pluginId, queryName, intervalMs) {
    const [state, setState] = useState({ data: null, error: null, loading: true });
    const mountedRef = useRef(true);
    const refreshRef = useRef(() => Promise.resolve(null));

    const refresh = useCallback(() => refreshRef.current(), []);

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
            timer = setTimeout(() => fetchOnce(true), intervalMs * multiplier);
        };

        const fetchOnce = async (scheduleNext) => {
            if (cancelled || !mountedRef.current) {
                return null;
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
                if (scheduleNext) schedule();
                return data;
            } catch (error) {
                if (cancelled || !mountedRef.current) {
                    return null;
                }
                consecutiveFailures++;
                const message = error instanceof Error ? error.message : String(error);
                setState(prev => ({ data: prev.data, error: message, loading: false }));
            }
            if (scheduleNext) schedule();
            return null;
        };

        refreshRef.current = () => fetchOnce(false);
        fetchOnce(true);

        return () => {
            cancelled = true;
            mountedRef.current = false;
            refreshRef.current = () => Promise.resolve(null);
            if (timer !== null) {
                clearTimeout(timer);
            }
        };
    }, [pluginId, queryName, intervalMs]);

    return { ...state, refresh };
}
