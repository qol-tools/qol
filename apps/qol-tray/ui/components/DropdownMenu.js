import { html } from '../lib/html.js';
import { useEffect, useRef } from 'preact/hooks';

export function DropdownMenuIcon() {
    return html`
        <svg class="plugin-menu-trigger-icon" viewBox="0 0 12 20" fill="currentColor" aria-hidden="true" focusable="false">
            <circle cx="6" cy="3.5" r="1.8" />
            <circle cx="6" cy="10" r="1.8" />
            <circle cx="6" cy="16.5" r="1.8" />
        </svg>
    `;
}

function containsTarget(ref, target) {
    return !!ref.current && ref.current.contains(target);
}

function menuClassName(className, openClassName, open) {
    if (!open) return className;
    return `${className} ${openClassName}`;
}

function stopPropagation(event) {
    event.stopPropagation();
}

function handleTriggerClick(event, open, onToggle, onClose) {
    event.preventDefault();
    event.stopPropagation();
    if (open) {
        onClose();
        return;
    }
    onToggle();
}

function useOutsideDismiss(open, triggerRef, menuRef, onClose) {
    useEffect(() => {
        if (!open) return;

        function onPointerDown(event) {
            if (containsTarget(triggerRef, event.target)) return;
            if (containsTarget(menuRef, event.target)) return;
            onClose();
        }

        document.addEventListener('pointerdown', onPointerDown);
        return () => document.removeEventListener('pointerdown', onPointerDown);
    }, [open, onClose]);
}

export function DropdownMenu({
    open,
    onToggle,
    onClose,
    triggerLabel,
    children,
    triggerClassName = 'plugin-menu-trigger',
    menuClass = 'plugin-context-menu',
    menuOpenClass = 'open',
    triggerContent = null
}) {
    const triggerRef = useRef(null);
    const menuRef = useRef(null);
    useOutsideDismiss(open, triggerRef, menuRef, onClose);
    const content = triggerContent || html`<${DropdownMenuIcon} />`;

    return html`
        <button
            type="button"
            class=${triggerClassName}
            ref=${triggerRef}
            onClick=${event => handleTriggerClick(event, open, onToggle, onClose)}
            aria-label=${triggerLabel}
            aria-haspopup="menu"
            aria-expanded=${open ? 'true' : 'false'}
        >
            ${content}
        </button>
        <div
            class=${menuClassName(menuClass, menuOpenClass, open)}
            ref=${menuRef}
            role="menu"
            onClick=${stopPropagation}
        >
            ${children}
        </div>
    `;
}
