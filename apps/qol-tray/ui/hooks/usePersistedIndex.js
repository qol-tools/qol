import { useEffect, useRef } from 'preact/hooks';
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

    function markRestored() { restoredRef.current = true; }

    return [value, setValue, ref, markRestored];
}
