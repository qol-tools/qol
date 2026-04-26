import { html } from '../html.js';
import { useCallback, useEffect, useRef } from 'preact/hooks';

export function CodeBlock({ text, onCopy, index, selected, onSelect }) {
    const ref = useRef(null);
    const copy = useCallback(() => {
        navigator.clipboard.writeText(text).then(() => {
            if (onCopy) onCopy();
        });
    }, [text, onCopy]);

    useEffect(() => {
        if (selected) ref.current?.focus?.({ preventScroll: true });
    }, [selected]);

    const focusValue = index;
    return html`
        <pre ref=${ref} class="code-block"
            data-selected-surface=""
            data-selected=${selected != null ? (selected ? 'true' : 'false') : undefined}
            data-index=${index != null ? String(index) : undefined}
            tabIndex="-1"
            onFocus=${onSelect ? () => onSelect(focusValue) : undefined}
            onKeyDown=${(e) => {
                if (e.key !== 'Enter' && e.key !== ' ') return;
                if (e.target !== ref.current) return;
                e.preventDefault();
                copy();
            }}
            onClick=${copy}
            title="Enter or click to copy. PgUp/PgDn/Home/End to scroll.">${text}</pre>
    `;
}
