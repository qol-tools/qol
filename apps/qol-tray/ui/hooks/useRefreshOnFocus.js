import { useEffect, useRef } from 'preact/hooks';

export function useRefreshOnFocus(callback) {
    const callbackRef = useRef(callback);
    callbackRef.current = callback;

    useEffect(() => {
        const onFocus = () => callbackRef.current();
        window.addEventListener('focus', onFocus);
        return () => window.removeEventListener('focus', onFocus);
    }, []);
}
