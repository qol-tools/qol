import { useEffect, useRef, useCallback } from 'preact/hooks';
import { useStateRef } from './useStateRef.js';

export function usePersistedIndex(storageKey, defaultValue = 0) {
    const [value, setValue, ref] = useStateRef(() => {
        const saved = parseInt(localStorage.getItem(storageKey), 10);
        return Number.isFinite(saved) && saved >= 0 ? saved : defaultValue;
    });
    const restoredRef = useRef(false);

    useEffect(() => {
        if (!restoredRef.current) return;
        localStorage.setItem(storageKey, String(value));
    }, [value, storageKey]);

    const markRestored = useCallback(() => { restoredRef.current = true; }, []);

    return [value, setValue, ref, markRestored];
}

export function usePersistedId(storageKey) {
    const [value, setValue, ref] = useStateRef(() => localStorage.getItem(storageKey));
    const restoredRef = useRef(false);

    useEffect(() => {
        if (!restoredRef.current) return;
        if (value == null) {
            localStorage.removeItem(storageKey);
            return;
        }
        localStorage.setItem(storageKey, value);
    }, [value, storageKey]);

    const markRestored = useCallback(() => { restoredRef.current = true; }, []);

    return [value, setValue, ref, markRestored];
}
