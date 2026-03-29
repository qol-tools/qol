import { useCallback } from 'preact/hooks';
import { dispatchKey, withShiftVariants } from '../../utils/keys.js';

export function useProfileKeyHandler({
    activateSelected,
    focusSelectedSurface,
    navigateInGrid,
}) {
    const handleTextInputNavigation = useCallback((event, active) => {
        if (event.key === 'Escape' || event.key === 'Enter') {
            event.preventDefault();
            focusSelectedSurface();
            return true;
        }
        const direction = arrowDirection(event.key);
        if (!direction) {
            return false;
        }
        if ((direction === 'left' || direction === 'right') && shouldKeepHorizontalCaret(event, active)) {
            return false;
        }
        event.preventDefault();
        navigateInGrid(direction);
        return true;
    }, [focusSelectedSurface, navigateInGrid]);

    const handleKey = useCallback((event) => {
        const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        if (isTextSurface(active)) {
            if (handleTextInputNavigation(event, active)) {
                return;
            }
            return;
        }
        dispatchKey(event, withShiftVariants({
            ArrowLeft: () => navigateInGrid('left'),
            ArrowRight: () => navigateInGrid('right'),
            ArrowUp: () => navigateInGrid('up'),
            ArrowDown: () => navigateInGrid('down'),
            h: () => navigateInGrid('left'),
            l: () => navigateInGrid('right'),
            k: () => navigateInGrid('up'),
            j: () => navigateInGrid('down'),
            Enter: activateSelected,
            ' ': activateSelected,
        }));
    }, [activateSelected, handleTextInputNavigation, navigateInGrid]);

    const isBlocking = useCallback(() => isTextSurface(document.activeElement), []);

    return {
        handleKey,
        isBlocking,
    };
}

function isTextSurface(element) {
    return element?.matches?.('[data-profile-editable], textarea, [contenteditable="true"]');
}

function arrowDirection(key) {
    if (key === 'ArrowUp') return 'up';
    if (key === 'ArrowDown') return 'down';
    if (key === 'ArrowLeft') return 'left';
    if (key === 'ArrowRight') return 'right';
    return null;
}

function shouldKeepHorizontalCaret(event, active) {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') {
        return false;
    }
    if (!(active instanceof HTMLInputElement) && !(active instanceof HTMLTextAreaElement)) {
        return false;
    }
    if (active.readOnly || active.disabled) {
        return false;
    }
    if (active.selectionStart === null || active.selectionEnd === null) {
        return false;
    }
    if (active.selectionStart !== active.selectionEnd) {
        return true;
    }
    if (event.key === 'ArrowLeft') {
        return active.selectionStart > 0;
    }
    return active.selectionEnd < active.value.length;
}
