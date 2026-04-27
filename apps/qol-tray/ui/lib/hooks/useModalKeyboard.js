import { useCallback, useLayoutEffect, useState } from 'preact/hooks';
import { modalFields } from '../components/ModalPreact.js';

export function useModalKeyboard({ onSave }) {
    const [selectedIndex, setSelectedIndex] = useState(0);

    useLayoutEffect(() => {
        const container = document.querySelector('.edit-modal-content');
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

        if (isEditing(active)) {
            if (e.key === 'Escape' || e.key === 'Enter') {
                e.preventDefault();
                active.closest('[data-selected-surface]')?.focus();
            }
            if (e.key === 'Enter' && e.ctrlKey) { e.preventDefault(); onSave(); }
            return;
        }

        if (e.key === 'Enter' && e.ctrlKey) { e.preventDefault(); onSave(); return; }
    }, [onSave]);

    const fieldProps = useCallback((index) => ({
        'data-selected-surface': '',
        'data-selected': index === selectedIndex ? 'true' : 'false',
        tabIndex: -1,
        onFocus: () => setSelectedIndex(index),
        onClick: activateFieldContent,
    }), [selectedIndex]);

    return { handleKey, fieldProps };
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
