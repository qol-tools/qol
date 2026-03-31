import { useCallback, useRef, useState } from 'preact/hooks';

/**
 * Generic keyboard-navigable surface system.
 *
 * Arrow navigation is handled by globalSurfaceNav. This hook provides:
 * - surfaceProps() for wiring up elements (data-selected-surface, onFocus sync)
 * - Enter/Space activation
 * - selectedIndex state
 */
export function useSurfaceKeyboard(containerSelector, { onActivate } = {}) {
    const [selectedIndex, setSelectedIndex] = useState(0);
    const selectedIndexRef = useRef(selectedIndex);
    selectedIndexRef.current = selectedIndex;

    const activate = useCallback(() => {
        if (!onActivate) {
            const el = document.querySelector(`${containerSelector}[data-index="${selectedIndexRef.current}"]`);
            if (el) el.click();
            return;
        }
        onActivate(selectedIndexRef.current);
    }, [containerSelector, onActivate]);

    const handleKey = useCallback((event) => {
        if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            activate();
        }
    }, [activate]);

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
    };
}
