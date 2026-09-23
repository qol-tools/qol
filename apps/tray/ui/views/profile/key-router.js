import { useCallback } from 'preact/hooks';

export function useProfileKeyHandler() {
    const handleKey = useCallback((event) => {
        const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        if (isTextSurface(active) && (event.key === 'Escape' || event.key === 'Enter')) {
            event.preventDefault();
            active.closest('[data-selected-surface]')?.focus();
        }
    }, []);

    const isBlocking = useCallback(() => isTextSurface(document.activeElement), []);

    return { handleKey, isBlocking };
}

function isTextSurface(element) {
    return element?.matches?.('[data-profile-editable], textarea, [contenteditable="true"]');
}
