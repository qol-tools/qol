import { html } from '../lib/html.js';
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
        <${Modal} open=${open} onClose=${onClose} size="sm">
            <div class="modal-body" style="padding: 16px;">
                <h3 style="margin: 0 0 8px;">${title}</h3>
                ${hint && html`<p class="text-muted" style="margin: 0 0 8px; font-size: 13px;">${hint}</p>`}
                <input
                    ref=${inputRef}
                    type="text"
                    class="input"
                    value=${value}
                    onInput=${(e) => setValue(e.target.value)}
                    onKeyDown=${handleKeyDown}
                    placeholder=${placeholder}
                    style="width: 100%;"
                />
            </div>
            <${ModalActions}
                onClose=${onClose}
                onSave=${handleSubmit}
                disabled=${!value.trim()}
            />
        <//>
    `;
}
