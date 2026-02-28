import { html } from '../lib/html.js';

export function Feedback({ feedback }) {
    if (!feedback) return null;
    return html`<div class="view-feedback ${feedback.type}">${feedback.message}</div>`;
}
