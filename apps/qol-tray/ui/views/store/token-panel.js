import { html } from '../../lib/html.js';

export function StoreTokenPanel({
    showTokenInput,
    hasToken,
    rateLimited,
    tokenInputRef,
    onSave,
    onDelete,
    onCancel,
    onShow
}) {
    if (showTokenInput) {
        return [
            html`
                <div class="token-input-container">
                    <input ref=${tokenInputRef} type="password" id="github-token-input"
                           placeholder="Paste GitHub token (no scopes needed)" />
                    <button class="btn btn-primary" onClick=${onSave}>Save</button>
                    ${hasToken && html`<button class="btn btn-ghost" onClick=${onDelete}>Remove Token</button>`}
                    <button class="btn btn-ghost" onClick=${onCancel}>Cancel</button>
                </div>
            `,
            html`
                <p class="token-help">
                    <a href="https://github.com/settings/tokens/new" target="_blank">Create token</a> — no scopes needed, just for rate limits
                </p>
            `
        ];
    }

    if (!rateLimited || hasToken) {
        return null;
    }

    return html`
        <div class="rate-limit-banner">
            <span>GitHub API rate limit reached.</span>
            <button class="btn btn-primary" onClick=${onShow}>
                Add GitHub Token
            </button>
        </div>
    `;
}
