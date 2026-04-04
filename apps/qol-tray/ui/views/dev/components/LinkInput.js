import { html } from '../../../lib/html.js';

export function LinkInput({ showLinkInput, linkPath, linkError, onInput, onConfirm, onCancel }) {
    if (!showLinkInput) return null;

    const onKeyDown = e => {
        if (e.key === 'Enter') onConfirm();
        if (e.key === 'Escape') onCancel();
    };

    return html`
        <div>
            <div class="link-input-row">
                <input type="text" id="link-path" placeholder="/path/to/plugin" value=${linkPath}
                    onInput=${e => onInput(e.target.value)} onKeyDown=${onKeyDown} />
                <button class="btn btn-sm btn-primary" onClick=${onConfirm}>Link</button>
                <button class="btn btn-sm btn-ghost" onClick=${onCancel}>Cancel</button>
            </div>
            ${linkError && html`<p class="error-msg">${linkError}</p>`}
        </div>
    `;
}
