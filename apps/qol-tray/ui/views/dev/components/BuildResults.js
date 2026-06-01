import { html } from '../../../lib/html.js';

export function BuildResults({ buildResults }) {
    if (!buildResults) return null;

    const failed = buildResults.filter(r => !r.success);
    const skipped = buildResults.filter(r => r.skipped);
    if (buildResults.length === 0 || skipped.length === buildResults.length) {
        return html`<span class="build-success">All linked plugins are up to date</span>`;
    }

    if (failed.length === 0) {
        const skippedText = skipped.length ? ` (${skipped.length} skipped)` : '';
        return html`<span class="build-success">Build succeeded${skippedText}</span>`;
    }

    const failedIds = failed.map(r => r.plugin_id).join(', ');
    return html`<span class="build-error">Build failed: ${failedIds}</span>`;
}
