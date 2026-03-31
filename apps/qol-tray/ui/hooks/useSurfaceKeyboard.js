import { useCallback, useRef, useState } from 'preact/hooks';
import { useGridNav } from './useGridNav.js';
import { dispatchKey, withShiftVariants } from '../utils/keys.js';

/**
 * Generic keyboard-navigable surface system.
 *
 * Any view or section can call this hook to get full arrow-key grid navigation,
 * Enter/Space activation, and surfaceProps() for wiring up elements.
 *
 * Usage:
 *   const nav = useSurfaceKeyboard('.my-container [data-selected-surface]');
 *   // Spread onto elements:  <div ...${nav.surfaceProps(i)}>
 *   // Pass to view keyboard: useRegisterViewKeyboard('myview', nav.handleKey);
 *   // Or call nav.handleKey(event) from a parent handler.
 */
export function useSurfaceKeyboard(containerSelector, { onActivate } = {}) {
    const [selectedIndex, setSelectedIndex] = useState(0);
    const selectedIndexRef = useRef(selectedIndex);
    selectedIndexRef.current = selectedIndex;

    const navigate = useGridNav(containerSelector, selectedIndexRef, setSelectedIndex);

    const activate = useCallback(() => {
        if (!onActivate) {
            const el = document.querySelector(`${containerSelector}[data-index="${selectedIndexRef.current}"]`);
            if (el) el.click();
            return;
        }
        onActivate(selectedIndexRef.current);
    }, [containerSelector, onActivate]);

    const handleKey = useCallback((event) => {
        dispatchKey(event, withShiftVariants({
            ArrowLeft: () => navigate('left'),
            ArrowRight: () => navigate('right'),
            ArrowUp: () => navigate('up'),
            ArrowDown: () => navigate('down'),
            h: () => navigate('left'),
            l: () => navigate('right'),
            k: () => navigate('up'),
            j: () => navigate('down'),
            Enter: activate,
            ' ': activate,
        }));
    }, [navigate, activate]);

    const surfaceProps = useCallback((index) => {
        const selected = index === selectedIndexRef.current;
        return {
            'data-selected-surface': '',
            'data-selected': selected ? 'true' : 'false',
            'data-index': String(index),
            onMouseDown: () => setSelectedIndex(index),
            onFocus: () => setSelectedIndex(index),
        };
    }, [selectedIndex]);

    return {
        selectedIndex,
        setSelectedIndex,
        handleKey,
        surfaceProps,
        navigate,
    };
}
