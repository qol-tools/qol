import { html } from '../lib/html.js';
import { useState, useCallback, useRef, useEffect } from 'preact/hooks';
import { Modal, ModalFooter } from './ModalPreact.js';

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
        if (e.key === 'Escape') { e.preventDefault(); onClose(); return; }
        if (e.key === 'Enter') { e.preventDefault(); handleSubmit(); }
    }, [handleSubmit, onClose]);

    return html`
        <${Modal} open=${open} onClose=${onClose} className="edit-modal">
            <div class="edit-modal-content" onKeyDown=${handleKeyDown}>
                <h3>${title}</h3>
                <div class="form-group">
                    <label>${hint || 'Path'}</label>
                    <input
                        ref=${inputRef}
                        type="text"
                        value=${value}
                        onInput=${(e) => setValue(e.target.value)}
                        placeholder=${placeholder}
                    />
                </div>
                <${ModalFooter} actions=${[
                    { label: 'Cancel', kbd: 'Esc', onClick: onClose },
                    { label: 'Confirm', kbd: 'Enter', variant: 'btn-primary', onClick: handleSubmit, disabled: !value.trim() },
                ]} />
            </div>
        <//>
    `;
}
