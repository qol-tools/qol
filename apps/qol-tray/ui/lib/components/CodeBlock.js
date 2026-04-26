import { html } from '../html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';

export function CodeBlock({ index, selected, onSelect, text }) {
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

    const className = ['code-block', scrollMode && 'is-scroll-mode'].filter(Boolean).join(' ');
    return html`
        <pre ref=${ref} class=${className}
            data-selected-surface=""
            data-selected=${selected != null ? (selected ? 'true' : 'false') : undefined}
            data-index=${index != null ? String(index) : undefined}
            data-scroll-surface-active=${scrollMode ? '' : undefined}
            tabIndex="-1"
            onFocus=${onSelect ? () => onSelect(index) : undefined}
            onClick=${enterScrollMode}
            onKeyDown=${(e) => {
                if (scrollMode) return;
                if (e.key !== 'Enter' && e.key !== ' ') return;
                e.preventDefault();
                enterScrollMode();
            }}
            title=${scrollMode ? 'Esc to exit scroll mode' : 'Enter or click to enter scroll mode (PgUp/PgDn/Arrows/Home/End)'}>${text}</pre>
    `;
}
