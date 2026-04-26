import { html } from '../html.js';
import { useCallback, useEffect, useRef } from 'preact/hooks';

export function CodeBlock({ text, onCopy, autoFocus = true }) {
    const ref = useRef(null);
    const copy = useCallback(() => {
        navigator.clipboard.writeText(text).then(() => {
            if (onCopy) onCopy();
        });
    }, [text, onCopy]);

    useEffect(() => {
        if (autoFocus) ref.current?.focus?.();
    }, [autoFocus, text]);

    return html`
        <pre ref=${ref} class="code-block" tabIndex="0" onClick=${copy} title="Click to copy (PgUp/PgDn/Arrows to scroll)">${text}</pre>
    `;
}
