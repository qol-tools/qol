import { useCallback } from 'preact/hooks';

export function useProfileKeyHandler({
    activateSelected,
    focusSelectedSurface,
}) {
    const handleKey = useCallback((event) => {
        const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        if (isTextSurface(active)) {
            if (event.key === 'Escape' || event.key === 'Enter') {
                event.preventDefault();
                focusSelectedSurface();
            }
            return;
        }
        if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            activateSelected();
        }
    }, [activateSelected, focusSelectedSurface]);

    const isBlocking = useCallback(() => isTextSurface(document.activeElement), []);

    return {
        handleKey,
        isBlocking,
    };
}

function isTextSurface(element) {
    return element?.matches?.('[data-profile-editable], textarea, [contenteditable="true"]');
}
