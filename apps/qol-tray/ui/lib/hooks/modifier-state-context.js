import { createContext } from 'preact';
import { useContext, useState, useEffect, useMemo } from 'preact/hooks';
import { html } from '../html.js';

const ModifierStateContext = createContext(null);

export function useModifierState() {
    return useContext(ModifierStateContext);
}

export function ModifierStateProvider({ children }) {
    const [ctrlHeld, setCtrlHeld] = useState(false);
    const [shiftHeld, setShiftHeld] = useState(false);

    useEffect(() => {
        const onKeyDown = (e) => {
            if (e.key === 'Control') setCtrlHeld(true);
            if (e.key === 'Shift') setShiftHeld(true);
        };
        const onKeyUp = (e) => {
            if (e.key === 'Control') setCtrlHeld(false);
            if (e.key === 'Shift') setShiftHeld(false);
        };
        const onBlur = () => {
            setCtrlHeld(false);
            setShiftHeld(false);
        };

        document.addEventListener('keydown', onKeyDown);
        document.addEventListener('keyup', onKeyUp);
        window.addEventListener('blur', onBlur);
        return () => {
            document.removeEventListener('keydown', onKeyDown);
            document.removeEventListener('keyup', onKeyUp);
            window.removeEventListener('blur', onBlur);
        };
    }, []);

    const value = useMemo(() => ({ ctrlHeld, shiftHeld }), [ctrlHeld, shiftHeld]);

    return html`<${ModifierStateContext.Provider} value=${value}>${children}<//>`;
}
