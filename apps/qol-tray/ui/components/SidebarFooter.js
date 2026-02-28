import { html } from '../lib/html.js';
import { clampPercent, formatDownloadingProgress, formatPhaseProgress } from '../utils/progress.js';

export function SidebarFooter({ version, updateState, isDevMode, onAction }) {
    if (!version) return null;
    const status = updateState?.status || 'idle';

    if (isDevMode) return renderDev(version, updateState, status, onAction);
    return renderStable(version, updateState, status, onAction);
}

function renderDev(version, state, status, onAction) {
    if (status === 'compiling') {
        const percent = clampPercent(state?.percent);
        const label = formatPhaseProgress(state?.phase, percent, 'Recompiling QoL Tray');
        return devProgress(version, label, percent, 'compiling');
    }
    if (status === 'downloading') {
        const percent = clampPercent(state?.percent);
        return devProgress(version, formatDownloadingProgress(percent), percent, '');
    }
    if (status === 'recompile_done') {
        return html`<div class="version-item is-dev update-done">
            <span class="version-main">v${version}<span class="version-tag">DEV</span></span>
            <span class="version-sub">Recompile complete</span>
        </div>`;
    }
    if (status === 'done') {
        return html`<div class="version-item is-dev update-done">
            <span class="version-main">v${version}<span class="version-tag">DEV</span></span>
            <span class="version-sub">Update complete</span>
        </div>`;
    }
    if (status === 'error') {
        const detail = state?.message ? `: ${state.message}` : '';
        return html`<div class="version-item is-dev" onClick=${() => onAction('dev-recompile')}>
            <span class="version-main">v${version}<span class="version-tag">DEV</span></span>
            <span class="version-sub">Action failed${detail}. Click to retry</span>
        </div>`;
    }
    // idle
    return html`<div class="version-item is-dev" onClick=${() => onAction('dev-recompile')}>
        <span class="version-main">v${version}<span class="version-tag">DEV</span></span>
        <span class="version-sub">Recompile QoL Tray</span>
    </div>`;
}

function renderStable(version, state, status, onAction) {
    if (status === 'downloading') {
        const percent = clampPercent(state?.percent);
        return html`<div class="version-item is-downloading">
            <div class="progress-fill" style="width: ${percent}%"></div>
            <span class="version-main">v${version}</span>
            <span class="version-sub">${formatDownloadingProgress(percent)}</span>
        </div>`;
    }
    if (status === 'done') {
        return html`<div class="version-item update-done">
            <span class="version-main">Restarting...</span>
            <span class="version-sub">v${version} installed</span>
        </div>`;
    }
    if (status === 'checking') {
        return html`<div class="version-item">
            <span class="version-main">v${version}</span>
            <span class="version-sub">Checking for updates...</span>
        </div>`;
    }
    if (status === 'available') {
        return html`<div class="version-item has-update" onClick=${() => onAction('self-update')}>
            <span class="version-main">v${state?.latest} available</span>
            <span class="version-sub">Click to update from v${version}</span>
        </div>`;
    }
    if (status === 'error') {
        return html`<div class="version-item" onClick=${() => onAction('check-update')}>
            <span class="version-main">v${version}</span>
            <span class="version-sub">Update failed. Click to retry</span>
        </div>`;
    }
    // idle / up-to-date
    return html`<div class="version-item" onClick=${() => onAction('check-update')}>
        <span class="version-main">v${version}</span>
        <span class="version-sub">Check for updates</span>
    </div>`;
}

function devProgress(version, label, percent, extraClass) {
    return html`<div class="version-item is-dev is-downloading ${extraClass}">
        <div class="progress-fill" style="width: ${percent}%"></div>
        <span class="version-main">v${version}<span class="version-tag">DEV</span></span>
        <span class="version-sub">${label}</span>
    </div>`;
}
