import { useEffect } from 'preact/hooks';

export function useScrollIntoView(selector, deps) {
    useEffect(() => {
        if (!selector) return;
        const el = document.querySelector(selector);
        if (el) el.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    }, deps);
}
