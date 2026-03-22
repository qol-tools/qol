import { html } from '../lib/html.js';
import { useCallback } from 'preact/hooks';

export function CodeBlock({ text, onCopy }) {
    const copy = useCallback(() => {
        navigator.clipboard.writeText(text).then(() => {
            if (onCopy) onCopy();
        });
    }, [text, onCopy]);

    return html`
        <pre class="code-block" onClick=${copy} title="Click to copy">${text}</pre>
    `;
}
