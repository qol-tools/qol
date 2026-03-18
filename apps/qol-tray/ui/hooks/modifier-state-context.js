import { createContext } from 'preact';
import { useContext, useState, useEffect, useMemo } from 'preact/hooks';
import { html } from '../lib/html.js';

const ModifierStateContext = createContext(null);

export function useModifierState() {
    return useContext(ModifierStateContext);
}

export function ModifierStateProvider({ children }) {
    const [ctrlHeld, setCtrlHeld] = useState(false);

    useEffect(() => {
        const onKeyDown = (e) => {
            if (e.key === 'Control') setCtrlHeld(true);
        };
        const onKeyUp = (e) => {
            if (e.key === 'Control') setCtrlHeld(false);
        };
        const onBlur = () => setCtrlHeld(false);

        document.addEventListener('keydown', onKeyDown);
        document.addEventListener('keyup', onKeyUp);
        window.addEventListener('blur', onBlur);
        return () => {
            document.removeEventListener('keydown', onKeyDown);
            document.removeEventListener('keyup', onKeyUp);
            window.removeEventListener('blur', onBlur);
        };
    }, []);

    const value = useMemo(() => ({ ctrlHeld }), [ctrlHeld]);

    return html`<${ModifierStateContext.Provider} value=${value}>${children}<//>`;
}
