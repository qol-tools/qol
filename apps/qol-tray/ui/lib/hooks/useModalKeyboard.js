import { useCallback, useLayoutEffect, useState } from 'preact/hooks';
import { modalFields } from '../components/ModalPreact.js';
import { resolveModalKeyAction } from './modal-key-action.js';

let activeModalContainer = null;
let warnedFallback = false;

/**
 * Registered by DiveEditorSubPage so useModalKeyboard (called in the parent
 * view's hook layer) can resolve the modal container without each caller
 * threading a ref through.
 */
export function setActiveModalContainer(ref) {
    activeModalContainer = ref || null;
}

export function useModalKeyboard({ onSave, onClose, containerRef } = {}) {
    const [selectedIndex, setSelectedIndex] = useState(0);

    useLayoutEffect(() => {
        const container = resolveContainer(containerRef);
        if (!container) return;
        const surfaces = getModalSurfaces(container);
        const surface = surfaces[Math.min(selectedIndex, surfaces.length - 1)];
        if (surface && !surface.contains(document.activeElement)) {
            surface.focus({ preventScroll: true });
        }
    }, [selectedIndex]);

    const handleKey = useCallback((e) => {
        const active = document.activeElement;
        if (active?.closest('.custom-select-list')) return;

        const action = resolveModalKeyAction({
            key: e.key,
            ctrlKey: !!e.ctrlKey,
            isEditing: isEditing(active),
            hasOnClose: !!onClose,
        });
        if (action === 'noop') return;
        e.preventDefault();
        if (action === 'blur-edit') { active.closest('[data-selected-surface]')?.focus(); return; }
        if (action === 'blur-edit-and-save') { active.closest('[data-selected-surface]')?.focus(); onSave(); return; }
        if (action === 'save') { onSave(); return; }
        if (action === 'close') { onClose(); return; }
    }, [onSave, onClose]);

    const fieldProps = useCallback((index) => ({
        'data-selected-surface': '',
        'data-selected': index === selectedIndex ? 'true' : 'false',
        tabIndex: -1,
        onFocus: () => setSelectedIndex(index),
        onClick: activateFieldContent,
    }), [selectedIndex]);

    return { handleKey, fieldProps };
}

function resolveContainer(containerRef) {
    if (containerRef?.current) return containerRef.current;
    if (activeModalContainer?.current) return activeModalContainer.current;
    const fallback = document.querySelector('.edit-modal-content');
    if (fallback && !warnedFallback) {
        warnedFallback = true;
        // eslint-disable-next-line no-console
        console.warn('useModalKeyboard: falling back to global .edit-modal-content selector. Wire a containerRef or render inside DiveEditorSubPage.');
    }
    return fallback;
}

function getModalSurfaces(container) {
    return Array.from(container.querySelectorAll('[data-selected-surface]'))
        .filter(el => !el.parentElement?.closest('[data-selected-surface]'));
}

function activateFieldContent(e) {
    if (e.target !== e.currentTarget) return;
    const el = e.currentTarget;
    const toggle = el.querySelector('[role="switch"]');
    if (toggle) { toggle.click(); return; }
    const trigger = el.querySelector('.custom-select-trigger');
    if (trigger) { trigger.click(); return; }
    const fields = modalFields(el);
    const target = fields[0];
    if (!target) return;
    if (target.readOnly) { target.click(); return; }
    target.focus();
    if (target.tagName === 'INPUT' && target.select) target.select();
}

function isEditing(el) {
    if (!el) return false;
    const tag = el.tagName;
    if (tag === 'TEXTAREA') return true;
    if (tag !== 'INPUT') return false;
    return !el.readOnly && !el.disabled;
}
