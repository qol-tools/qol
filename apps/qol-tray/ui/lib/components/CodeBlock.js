import { html } from '../html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';

export function CodeBlock({ index, selected, onSelect, onSecondaryActivate, text, secondaryLabel }) {
    const ref = useRef(null);
    const [scrollMode, setScrollMode] = useState(false);

    const enterScrollMode = useCallback(() => {
        setScrollMode(true);
        ref.current?.focus?.({ preventScroll: true });
    }, []);

    useEffect(() => {
        if (!scrollMode) return;
        const el = ref.current;
        if (!el) return;
        const exit = () => setScrollMode(false);
        el.addEventListener('exit-scroll-mode', exit);
        el.addEventListener('blur', exit);
        return () => {
            el.removeEventListener('exit-scroll-mode', exit);
            el.removeEventListener('blur', exit);
        };
    }, [scrollMode]);

    const handleClick = (e) => {
        if (e.shiftKey && onSecondaryActivate) {
            onSecondaryActivate(e);
            return;
        }
        enterScrollMode();
    };

    const handleKeyDown = (e) => {
        if (scrollMode) return;
        if (e.key !== 'Enter' && e.key !== ' ') return;
        e.preventDefault();
        if (e.shiftKey && onSecondaryActivate) {
            onSecondaryActivate(e);
            return;
        }
        enterScrollMode();
    };

    const className = ['code-block', scrollMode && 'is-scroll-mode'].filter(Boolean).join(' ');
    const tip = scrollMode
        ? 'Esc to exit scroll mode'
        : onSecondaryActivate
            ? `Enter to scroll, Shift+Enter to ${secondaryLabel || 'open externally'}`
            : 'Enter to enter scroll mode (PgUp/PgDn/Arrows/Home/End)';
    return html`
        <pre ref=${ref} class=${className}
            data-selected-surface=""
            data-selected=${selected != null ? (selected ? 'true' : 'false') : undefined}
            data-index=${index != null ? String(index) : undefined}
            data-secondary-label=${onSecondaryActivate ? (secondaryLabel || 'Open in editor') : undefined}
            data-scroll-surface-active=${scrollMode ? '' : undefined}
            tabIndex="-1"
            onFocus=${onSelect ? () => onSelect(index) : undefined}
            onClick=${handleClick}
            onKeyDown=${handleKeyDown}
            title=${tip}>${text}</pre>
    `;
}
