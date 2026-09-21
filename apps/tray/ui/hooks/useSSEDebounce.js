import { useRef, useCallback } from 'preact/hooks';
import { useSSE } from './useSSE.js';

export function useSSEDebounce(eventType, callback, delay = 100) {
    const timerRef = useRef(null);
    const callbackRef = useRef(callback);
    callbackRef.current = callback;

    useSSE(useCallback((event) => {
        if (event.type !== eventType) return;
        if (timerRef.current) clearTimeout(timerRef.current);
        timerRef.current = setTimeout(() => {
            timerRef.current = null;
            callbackRef.current(event);
        }, delay);
    }, [eventType, delay]));
}
