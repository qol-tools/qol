import { useState, useCallback, useLayoutEffect } from 'preact/hooks';
import { modalFields } from '../components/ModalPreact.js';

function getSurfaces() {
    const container = document.querySelector('.edit-modal');
    if (!container) return [];
    return Array.from(container.querySelectorAll('[data-selected-surface]'));
}

export function useModalKeyboard({ onSave, onClose }) {
    const [selectedIndex, setSelectedIndex] = useState(0);

    useLayoutEffect(() => {
        const surfaces = getSurfaces();
        const surface = surfaces[selectedIndex];
        if (!surface) return;
        if (surface.contains(document.activeElement)) return;
        surface.focus();
    });

    const handleKey = useCallback((e) => {
        const surfaces = getSurfaces();
        if (surfaces.length === 0) return;

        const surface = surfaces.find(s => s.getAttribute('data-selected') === 'true');
        const active = document.activeElement;

        if (active?.closest('.custom-select-list')) return;

        if (isEditing(active)) {
            if (e.key === 'Escape' || e.key === 'Enter') {
                e.preventDefault();
                surface?.focus();
            }
            if (e.key === 'Enter' && e.ctrlKey) { e.preventDefault(); onSave(); }
            return;
        }

        if (e.key === 'Enter' && e.ctrlKey) { e.preventDefault(); onSave(); return; }

        if (e.key === 'ArrowDown' || e.key === 'j') {
            e.preventDefault();
            navigate(surfaces, 1, setSelectedIndex);
            return;
        }
        if (e.key === 'ArrowUp' || e.key === 'k') {
            e.preventDefault();
            navigate(surfaces, -1, setSelectedIndex);
            return;
        }
        if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            activate(surface);
            return;
        }
    }, [onSave, onClose]);

    const fieldProps = useCallback((index) => ({
        'data-selected-surface': '',
        'data-selected': index === selectedIndex ? 'true' : 'false',
        onMouseDown: () => setSelectedIndex(index),
        onFocus: () => setSelectedIndex(index),
    }), [selectedIndex]);

    return { selectedIndex, setSelectedIndex, handleKey, fieldProps };
}

function navigate(surfaces, delta, setSelectedIndex) {
    setSelectedIndex(prev => {
        const next = Math.max(0, Math.min(surfaces.length - 1, prev + delta));
        surfaces[next]?.focus();
        return next;
    });
}

function activate(surface) {
    if (!surface) return;
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
