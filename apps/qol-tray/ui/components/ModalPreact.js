import { html } from '../lib/html.js';
import { useCallback } from 'preact/hooks';

export function Modal({ open, onClose, className, children }) {
    const handleBackdrop = useCallback((e) => {
        if (e.target === e.currentTarget) onClose();
    }, [onClose]);

    if (!open) return null;
    return html`<div class=${className} onClick=${handleBackdrop}>${children}</div>`;
}
