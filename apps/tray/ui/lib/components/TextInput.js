import { html } from '../html.js';

export function TextInput({ type = 'text', className, onSubmit, onCancel, onKeyDown, inputRef, ...rest }) {
    const handleKeyDown = (e) => {
        if (e.key === 'Enter' && onSubmit) onSubmit(e);
        if (e.key === 'Escape' && onCancel) onCancel(e);
        onKeyDown?.(e);
    };
    return html`<input type=${type} class=${className} ref=${inputRef} onKeyDown=${handleKeyDown} ...${rest} />`;
}
