import { useEffect } from 'preact/hooks';

const NAV_KEYS = new Set(['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight']);
let navKeyPressed = false;

export function useScrollFollow() {
    useEffect(() => {
        const onKeyDown = (e) => {
            if (NAV_KEYS.has(e.key)) navKeyPressed = true;
        };
        const observer = new MutationObserver(mutations => {
            if (!navKeyPressed) return;
            for (const mutation of mutations) {
                if (mutation.attributeName !== 'class') continue;
                const el = mutation.target;
                if (el.classList.contains('selected')) {
                    navKeyPressed = false;
                    el.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
                    return;
                }
            }
        });
        document.addEventListener('keydown', onKeyDown, true);
        observer.observe(document.body, {
            attributes: true,
            attributeFilter: ['class'],
            subtree: true
        });
        return () => {
            document.removeEventListener('keydown', onKeyDown, true);
            observer.disconnect();
        };
    }, []);
}
