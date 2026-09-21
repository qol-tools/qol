import { useRef, useCallback } from 'preact/hooks';

export function useAsyncToken() {
    const tokenRef = useRef(0);
    const next = useCallback(() => ++tokenRef.current, []);
    const isCurrent = useCallback((token) => token === tokenRef.current, []);
    return [next, isCurrent];
}
