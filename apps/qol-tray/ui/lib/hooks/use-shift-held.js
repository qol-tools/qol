import { useEffect } from 'preact/hooks';

const ATTR = 'data-shift-held';

export function useShiftHeld() {
    useEffect(() => {
        const set = () => document.body.setAttribute(ATTR, '');
        const clear = () => document.body.removeAttribute(ATTR);
        const onKeyDown = (event) => { if (event.key === 'Shift') set(); };
        const onKeyUp = (event) => { if (event.key === 'Shift') clear(); };
        window.addEventListener('keydown', onKeyDown, true);
        window.addEventListener('keyup', onKeyUp, true);
        window.addEventListener('blur', clear);
        return () => {
            window.removeEventListener('keydown', onKeyDown, true);
            window.removeEventListener('keyup', onKeyUp, true);
            window.removeEventListener('blur', clear);
            clear();
        };
    }, []);
}
