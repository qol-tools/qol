import { html } from '../lib/html.js';
import { useState } from '../lib/hooks.module.js';
import { useSSE } from '../hooks/useSSE.js';

export function BootHealedBanner() {
    const [report, setReport] = useState(null);
    const [dismissed, setDismissed] = useState(false);

    useSSE((event) => {
        if (event?.type === 'boot_target_healed') {
            setReport(event.report);
            setDismissed(false);
        }
    });

    if (!report || dismissed) return null;
    if (!report.events || report.events.length === 0) return null;

    const hasFailure = (report.failures?.length ?? 0) > 0;
    const tone = hasFailure ? 'danger' : 'info';
    const summary = hasFailure
        ? 'qol-tray detected boot target drift but could not repair it (autostart write failed). Run doctor to retry.'
        : describeHeal(report);

    return html`
        <div class=${'boot-healed-banner banner-' + tone} role="status" aria-live="polite">
            <span class="banner-text">${summary}</span>
            <a href="#/doctor" class="banner-action">Open doctor</a>
            <button
                class="banner-dismiss"
                type="button"
                onClick=${() => setDismissed(true)}
                aria-label="Dismiss"
            >×</button>
        </div>
    `;
}

function describeHeal(report) {
    const actions = report.actions || [];
    const cleared = actions.find((a) => a.kind === 'cleared_selection');
    const wrote = actions.find((a) => a.kind === 'wrote_autostart');
    const branch = cleared?.branch;
    if (branch && wrote) {
        return `qol-tray repaired a stale boot target on startup. Branch "${branch}" no longer existed; autostart was reset to ${wrote.binary}.`;
    }
    if (wrote) {
        return `qol-tray re-aligned the autostart target to ${wrote.binary}.`;
    }
    if (branch) {
        return `qol-tray cleared a stale dev selection (branch "${branch}").`;
    }
    return 'qol-tray observed boot-target drift on startup.';
}
