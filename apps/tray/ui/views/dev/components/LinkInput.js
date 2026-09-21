import { html } from '../../../lib/html.js';
import { TextInput } from '../../../lib/components/TextInput.js';
import { Button } from '../../../lib/components/Button.js';

export function LinkInput({ showLinkInput, linkPath, linkError, onInput, onConfirm, onCancel }) {
    if (!showLinkInput) return null;

    return html`
        <div>
            <div class="link-input-row">
                <${TextInput} id="link-path" placeholder="/path/to/plugin" value=${linkPath}
                    onInput=${e => onInput(e.target.value)} onSubmit=${onConfirm} onCancel=${onCancel} />
                <${Button} small variant="btn-primary" onActivate=${onConfirm}>Link<//>
                <${Button} small variant="btn-ghost" onActivate=${onCancel}>Cancel<//>
            </div>
            ${linkError && html`<p class="error-msg">${linkError}</p>`}
        </div>
    `;
}
