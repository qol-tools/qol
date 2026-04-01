import { html } from '../lib/html.js';
import { useCallback, useEffect, useRef } from 'preact/hooks';
import { surfaceDepth } from '../lib/surface-traits.js';

const FOCUSABLE = 'input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), button:not([disabled]), .custom-select-trigger:not([disabled]), [tabindex]:not([disabled]):not([tabindex="-1"])';

export function Modal({ open, onClose, dismissOnBackdrop, size, className, children }) {
    const containerRef = useRef(null);

    const handleBackdrop = useCallback((e) => {
        if (dismissOnBackdrop && e.target === e.currentTarget) onClose();
    }, [onClose, dismissOnBackdrop]);

    useEffect(() => {
        const prev = document.activeElement instanceof HTMLElement && document.activeElement !== document.body
            ? document.activeElement : null;
        const el = containerRef.current;
        if (el) {
            if (prev) el.setAttribute('data-surface-depth-base', String(surfaceDepth(prev)));
            const surface = el.querySelector('[data-selected-surface]');
            if (surface) surface.focus();
            else el.querySelector(FOCUSABLE)?.focus();
        }
        return () => { if (prev?.isConnected) prev.focus(); };
    }, []);

    useEffect(() => {
        if (!open || !onClose) return;
        const handler = (e) => {
            if (e.key !== 'Escape' || e.defaultPrevented) return;
            const el = containerRef.current;
            if (el) {
                const rect = el.getBoundingClientRect();
                if (rect.width === 0 && rect.height === 0) return;
            }
            e.preventDefault();
            e.stopPropagation();
            onClose();
        };
        window.addEventListener('keydown', handler);
        return () => window.removeEventListener('keydown', handler);
    }, [open, onClose]);

    if (!open) return null;
    const sizeClass = size ? `modal-${size}` : '';
    const cls = [className, sizeClass].filter(Boolean).join(' ');
    return html`<div class=${cls} ref=${containerRef} data-surface-container="" onClick=${handleBackdrop}>${children}</div>`;
}

export function modalFields(container) {
    if (!container) return [];
    return Array.from(container.querySelectorAll(FOCUSABLE));
}

export function ModalFooter({ actions }) {
    const ref = useRef(null);
    useEffect(() => {
        const bindings = actions
            .filter(a => a.kbd && a.onClick && !a.disabled)
            .map(a => ({ key: normalizeKbd(a.kbd), handler: a.onClick }))
            .filter(b => b.key);
        if (bindings.length === 0) return;
        const handler = (e) => {
            if (e.defaultPrevented) return;
            if (!isElementVisible(ref.current)) return;
            for (const b of bindings) {
                if (matchesKbd(e, b.key)) {
                    e.preventDefault();
                    e.stopPropagation();
                    b.handler();
                    return;
                }
            }
        };
        window.addEventListener('keydown', handler);
        return () => window.removeEventListener('keydown', handler);
    }, [actions]);

    return html`
        <div class="modal-footer-actions" ref=${ref}>
            ${actions.map(a => html`
                <button key=${a.label} class="btn ${a.variant || 'btn-ghost'}"
                    data-selected-surface=""
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

function normalizeKbd(kbd) {
    if (!kbd) return null;
    const lower = kbd.toLowerCase().trim();
    if (lower === 'esc' || lower === 'escape') return null; // Modal handles ESC directly
    return lower;
}

function matchesKbd(event, kbd) {
    if (!kbd) return false;
    const parts = kbd.split('+');
    const key = parts[parts.length - 1];
    const needsCtrl = parts.includes('ctrl');
    const needsShift = parts.includes('shift');
    if (needsCtrl && !(event.ctrlKey || event.metaKey)) return false;
    if (!needsCtrl && (event.ctrlKey || event.metaKey)) return false;
    if (needsShift && !event.shiftKey) return false;
    if (isEditing()) return false;
    return event.key.toLowerCase() === key;
}

function isElementVisible(el) {
    if (!el) return false;
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
}

function isEditing() {
    const a = document.activeElement;
    if (!a) return false;
    if (a.tagName === 'INPUT' && a.type !== 'button' && a.type !== 'checkbox' && a.type !== 'radio') return true;
    if (a.tagName === 'TEXTAREA') return true;
    if (a.contentEditable === 'true') return true;
    return false;
}
