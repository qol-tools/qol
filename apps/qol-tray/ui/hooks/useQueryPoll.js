import { useEffect, useRef, useState } from 'preact/hooks';

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
                    setState({ data, error: null, loading: false });
                }
            } catch (error) {
                if (cancelled || !mountedRef.current) {
                    return;
                }
                const message = error instanceof Error ? error.message : String(error);
                setState(prev => ({ data: prev.data, error: message, loading: false }));
            }
        };

        fetchOnce();
        if (intervalMs > 0) {
            timer = setInterval(fetchOnce, intervalMs);
        }

        return () => {
            cancelled = true;
            mountedRef.current = false;
            if (timer !== null) {
                clearInterval(timer);
            }
        };
    }, [pluginId, queryName, intervalMs]);

    return state;
}
