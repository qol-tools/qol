import { html } from '../html.js';
import { useState, useCallback, useRef, useEffect } from 'preact/hooks';
import { Modal, ModalActions } from './ModalPreact.js';

export function PathPromptModal({ open, onClose, onSubmit, title, placeholder, hint }) {
    const [value, setValue] = useState('');
    const inputRef = useRef(null);

    useEffect(() => {
        if (open && inputRef.current) inputRef.current.focus();
    }, [open]);

    const handleSubmit = useCallback(() => {
        const trimmed = value.trim();
        if (trimmed) onSubmit(trimmed);
    }, [value, onSubmit]);

    const handleKeyDown = useCallback((e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            handleSubmit();
        }
    }, [handleSubmit]);

    return html`
        <${Modal} open=${open} onClose=${onClose} size="sm" className="path-prompt-modal" dismissOnBackdrop=${true}>
            <div class="path-prompt-dialog">
                <div class="path-prompt-body">
                    <h3 class="path-prompt-title">${title}</h3>
                    ${hint && html`<p class="path-prompt-hint">${hint}</p>`}
                    <input
                        ref=${inputRef}
                        type="text"
                        class="path-prompt-input"
                        value=${value}
                        onInput=${(e) => setValue(e.target.value)}
                        onKeyDown=${handleKeyDown}
                        placeholder=${placeholder}
                    />
                </div>
                <${ModalActions}
                    onClose=${onClose}
                    onSave=${handleSubmit}
                    disabled=${!value.trim()}
                />
            </div>
        <//>
    `;
}
