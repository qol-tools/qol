import { useState, useEffect } from 'preact/hooks';
import { getModifierState, subscribeModifiers } from '../modifier-state.js';

export function useModifierState() {
    const [state, setState] = useState(getModifierState);
    useEffect(() => {
        const sync = () => setState(getModifierState());
        const unsubscribe = subscribeModifiers(sync);
        sync();
        return unsubscribe;
    }, []);
    return { ctrlHeld: state.ctrl, shiftHeld: state.shift };
}
