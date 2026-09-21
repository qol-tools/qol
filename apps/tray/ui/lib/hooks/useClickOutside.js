import { useEffect } from 'preact/hooks';

export function useClickOutside(ref, active, callback) {
    useEffect(() => {
        if (!active) return;
        const handler = (e) => {
            if (ref.current?.contains(e.target)) return;
            callback();
        };
        document.addEventListener('pointerdown', handler);
        return () => document.removeEventListener('pointerdown', handler);
    }, [ref, active, callback]);
}
