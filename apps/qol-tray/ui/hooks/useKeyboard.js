import { useEffect } from 'preact/hooks';

export function useKeyboard(handler) {
    useEffect(() => {
        document.addEventListener('keydown', handler);
        return () => document.removeEventListener('keydown', handler);
    }, [handler]);
}
