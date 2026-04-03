import { html } from '../../../lib/html.js';
import { Button } from '../../../components/Button.js';

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
                <${Button} variant="btn-primary" small onActivate=${onConfirm}>Link<//>
                <${Button} variant="btn-ghost" small onActivate=${onCancel}>Cancel<//>
            </div>
            ${linkError && html`<p class="error-msg">${linkError}</p>`}
        </div>
    `;
}
