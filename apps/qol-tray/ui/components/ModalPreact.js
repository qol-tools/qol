import { html } from '../lib/html.js';
import { useCallback, useLayoutEffect, useRef } from 'preact/hooks';

const FOCUSABLE = 'input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), button:not([disabled]), .custom-select-trigger:not([disabled]), [tabindex]:not([disabled]):not([tabindex="-1"])';

export function Modal({ open, onClose, dismissOnBackdrop, size, className, children }) {
    const containerRef = useRef(null);

    const handleBackdrop = useCallback((e) => {
        if (dismissOnBackdrop && e.target === e.currentTarget) onClose();
    }, [onClose, dismissOnBackdrop]);

    useLayoutEffect(() => {
        if (!open) return;
        const el = containerRef.current;
        if (!el) return;
        const surface = el.querySelector('[data-selected-surface][data-selected="true"]');
        if (surface) { surface.focus(); return; }
        const first = el.querySelector(FOCUSABLE);
        first?.focus();
    }, [open]);

    if (!open) return null;
    const sizeClass = size ? `modal-${size}` : '';
    const cls = [className, sizeClass].filter(Boolean).join(' ');
    return html`<div class=${cls} ref=${containerRef} onClick=${handleBackdrop}>${children}</div>`;
}

export function modalFields(container) {
    if (!container) return [];
    return Array.from(container.querySelectorAll(FOCUSABLE));
}

export function ModalFooter({ actions }) {
    return html`
        <div class="modal-footer-actions">
            ${actions.map(a => html`
                <button key=${a.label} class="btn ${a.variant || 'btn-ghost'}"
                    onClick=${a.onClick} disabled=${a.disabled}>
                    ${a.label}${a.kbd && html` <kbd>${a.kbd}</kbd>`}
                </button>
            `)}
        </div>
    `;
}

export function ModalActions({ onClose, onSave, disabled }) {
    return html`<${ModalFooter} actions=${[
        { label: 'Cancel', kbd: 'Esc', onClick: onClose },
        { label: 'Save', kbd: 'Ctrl+Enter', variant: 'btn-primary', onClick: !disabled ? onSave : undefined, disabled },
    ]} />`;
}
