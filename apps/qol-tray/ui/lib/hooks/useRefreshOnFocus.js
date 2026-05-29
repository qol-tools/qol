import { useEffect, useRef } from 'preact/hooks';

export function useRefreshOnFocus(callback, { minIntervalMs = 0 } = {}) {
    const callbackRef = useRef(callback);
    callbackRef.current = callback;
    const lastRef = useRef(0);

    useEffect(() => {
        const onFocus = () => {
            if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
            const now = Date.now();
            if (minIntervalMs > 0 && now - lastRef.current < minIntervalMs) return;
            lastRef.current = now;
            callbackRef.current();
        };
        window.addEventListener('focus', onFocus);
        return () => window.removeEventListener('focus', onFocus);
    }, [minIntervalMs]);
}
