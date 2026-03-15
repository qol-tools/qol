import { createContext } from 'preact';
import { useCallback, useContext, useEffect, useMemo, useRef } from 'preact/hooks';
import { html } from '../../lib/html.js';

const ViewKeyboardContext = createContext(null);

export function useViewKeyboardContext() {
    const context = useContext(ViewKeyboardContext);
    if (context) {
        return context;
    }
    return {
        getViewKeyboard: () => null,
        registerViewKeyboard: () => () => {},
    };
}

export function ViewKeyboardProvider({ children }) {
    const registryRef = useRef(new Map());

    const registerViewKeyboard = useCallback((viewId, keyboard) => {
        registryRef.current.set(viewId, keyboard);
        return () => {
            if (registryRef.current.get(viewId) !== keyboard) {
                return;
            }
            registryRef.current.delete(viewId);
        };
    }, []);

    const getViewKeyboard = useCallback((viewId) => {
        if (!viewId) {
            return null;
        }
        return registryRef.current.get(viewId) || null;
    }, []);

    const value = useMemo(() => ({
        getViewKeyboard,
        registerViewKeyboard,
    }), [getViewKeyboard, registerViewKeyboard]);

    return html`<${ViewKeyboardContext.Provider} value=${value}>${children}<//>`;
}

export function useRegisterViewKeyboard(viewId, handleKey, isBlocking = null) {
    const { registerViewKeyboard } = useViewKeyboardContext();

    useEffect(() => {
        if (!viewId) {
            return;
        }
        return registerViewKeyboard(viewId, { handleKey, isBlocking });
    }, [viewId, handleKey, isBlocking, registerViewKeyboard]);
}
