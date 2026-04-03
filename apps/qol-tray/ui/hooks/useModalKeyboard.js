import { useCallback, useLayoutEffect } from 'preact/hooks';
import { modalFields } from '../components/ModalPreact.js';

function getSurfaces() {
    const container = document.querySelector('.edit-modal');
    if (!container) return [];
    return Array.from(container.querySelectorAll('[data-selected-surface]'))
        .filter(el => !el.parentElement?.closest('[data-selected-surface]'));
}

export function useModalKeyboard({ onSave }) {
    // Sync data-selected with focus. Runs on every render to counteract
    // Preact removing data-selected from Button surfaces, and listens for
    // focusin to track when globalSurfaceNav moves focus via .focus().
    useLayoutEffect(() => {
        const container = document.querySelector('.edit-modal');
        if (!container) return;
        const sync = () => {
            const surfaces = getSurfaces();
            for (const s of surfaces) {
                const focused = s === document.activeElement || s.contains(document.activeElement);
                s.setAttribute('data-selected', focused ? 'true' : 'false');
            }
        };
        sync();
        container.addEventListener('focusin', sync);
        return () => container.removeEventListener('focusin', sync);
    });

    // Modal-specific key handling. Arrow keys are NOT handled here —
    // globalSurfaceNav provides spatial navigation automatically.
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

        if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            const surface = active?.closest('[data-selected-surface]');
            activate(surface);
            return;
        }
    }, [onSave]);

    const fieldProps = useCallback((_index) => ({
        'data-selected-surface': '',
        tabIndex: -1,
    }), []);

    return { handleKey, fieldProps };
}

function activate(surface) {
    if (!surface) return;
    if (surface.tagName === 'BUTTON') { surface.click(); return; }
    const toggle = surface.querySelector('[role="switch"]');
    if (toggle) { toggle.click(); return; }
    const fields = modalFields(surface);
    const target = fields[0];
    if (!target) return;
    if (target.classList.contains('custom-select-trigger') || target.readOnly) {
        target.click();
        return;
    }
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
