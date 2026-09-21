import { html } from '../html.js';
import { Alert } from './StatusIndicators.js';
import { Button } from './Button.js';

const PROVIDER_LABELS = {
    github: 'GitHub',
};

export function AuthHealthBanner({ issue, busy, onReauthorize }) {
    if (!issue || issue.kind !== 'insufficient_scope') return null;
    const providerLabel = PROVIDER_LABELS[issue.provider] || issue.provider;
    const missing = Array.isArray(issue.missing) ? issue.missing : [];
    const scopeNames = missing.map(m => m.wire_name).filter(Boolean).join(', ');
    return html`
        <${Alert} variant="warning">
            <div class="auth-health-banner">
                <div class="auth-health-banner-body">
                    <strong>${providerLabel} reauthorization required.</strong>
                    ${' '}
                    Your stored ${providerLabel} credential is missing
                    ${scopeNames ? html` scope <code>${scopeNames}</code>.` : ' required scope.'}
                    <ul class="auth-health-banner-reasons">
                        ${missing.map(m => html`<li key=${m.wire_name}>${m.reason}</li>`)}
                    </ul>
                </div>
                <div class="auth-health-banner-action">
                    <${Button}
                        variant="primary"
                        small=${true}
                        disabled=${busy}
                        onClick=${onReauthorize}
                    >${busy ? 'Reauthorizing...' : 'Reauthorize'}<//>
                </div>
            </div>
        <//>
    `;
}
