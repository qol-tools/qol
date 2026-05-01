import { useEffect, useRef } from 'preact/hooks';

export function useKeyboard(handler) {
    const handlerRef = useRef(handler);
    handlerRef.current = handler;

    useEffect(() => {
        const listener = (e) => handlerRef.current(e);
        document.addEventListener('keydown', listener);
        return () => document.removeEventListener('keydown', listener);
    }, []);
}
