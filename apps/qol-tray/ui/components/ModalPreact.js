import { html } from '../lib/html.js';
import { useCallback } from 'preact/hooks';

export function Modal({ open, onClose, dismissOnBackdrop, className, children }) {
    const handleBackdrop = useCallback((e) => {
        if (dismissOnBackdrop && e.target === e.currentTarget) onClose();
    }, [onClose, dismissOnBackdrop]);

    if (!open) return null;
    return html`<div class=${className} onClick=${handleBackdrop}>${children}</div>`;
}

export function ModalActions({ onClose, onSave, cancelTabIndex, saveTabIndex }) {
    return html`
        <div class="modal-buttons">
            <button class="btn btn-ghost modal-cancel" tabindex=${cancelTabIndex} onClick=${onClose}>Cancel <kbd>Esc</kbd></button>
            <button class="btn btn-primary modal-save" tabindex=${saveTabIndex} onClick=${onSave}>Save <kbd>Ctrl+Enter</kbd></button>
        </div>
    `;
}
