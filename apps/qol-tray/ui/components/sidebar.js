import { clampPercent, formatDownloadingProgress, formatPhaseProgress } from '../utils/progress.js';

const LABELS = {
    plugins: 'Plugins',
    store: 'Store',
    hotkeys: 'Hotkeys',
    'task-runner': 'Task Runner',
    dev: 'Developer'
};

export function render(
    activeViewId,
    viewOrder = ['plugins', 'store', 'hotkeys'],
    version = null,
    updateState = null,
    isDevMode = false
) {
    const items = viewOrder.map(id => `
        <div class="sidebar-item ${id === activeViewId ? 'active' : ''}" data-view="${id}">
            ${LABELS[id] || id}
        </div>
    `).join('');

    const versionHtml = version
        ? `<div class="sidebar-version">${renderVersionFooter(version, updateState, isDevMode)}</div>`
        : '';

    return `<div class="sidebar-nav">${items}</div>${versionHtml}`;
}

function renderDevProgress(version, label, percent, extraClass = '') {
    const classes = ['version-item', 'is-dev', 'is-downloading'];
    if (extraClass) classes.push(extraClass);
    return `<div class="${classes.join(' ')}">
                <div class="progress-fill" style="width: ${percent}%"></div>
                <span class="version-main">v${version}<span class="version-tag">DEV</span></span>
                <span class="version-sub">${label}</span>
            </div>`;
}

const DEV_FOOTER_RENDERERS = {
    compiling(version, state) {
        const percent = clampPercent(state?.percent);
        const label = formatPhaseProgress(state?.phase, percent, 'Recompiling QoL Tray');
        return renderDevProgress(version, label, percent, 'compiling');
    },
    downloading(version, state) {
        const percent = clampPercent(state?.percent);
        const label = formatDownloadingProgress(percent);
        return renderDevProgress(version, label, percent);
    },
    recompile_done(version) {
        return `<div class="version-item is-dev update-done">
                    <span class="version-main">v${version}<span class="version-tag">DEV</span></span>
                    <span class="version-sub">Recompile complete</span>
                </div>`;
    },
    done(version) {
        return `<div class="version-item is-dev update-done">
                    <span class="version-main">v${version}<span class="version-tag">DEV</span></span>
                    <span class="version-sub">Update complete</span>
                </div>`;
    },
    error(version, state) {
        const detail = state?.message ? `: ${state.message}` : '';
        return `<div class="version-item is-dev" data-action="dev-recompile">
                    <span class="version-main">v${version}<span class="version-tag">DEV</span></span>
                    <span class="version-sub">Action failed${detail}. Click to retry</span>
                </div>`;
    },
    idle(version) {
        return `<div class="version-item is-dev" data-action="dev-recompile">
                    <span class="version-main">v${version}<span class="version-tag">DEV</span></span>
                    <span class="version-sub">Recompile QoL Tray</span>
                </div>`;
    }
};

const STABLE_FOOTER_RENDERERS = {
    downloading(version, state) {
        const percent = clampPercent(state?.percent);
        const label = formatDownloadingProgress(percent);
        return `<div class="version-item is-downloading">
                    <div class="progress-fill" style="width: ${percent}%"></div>
                    <span class="version-main">v${version}</span>
                    <span class="version-sub">${label}</span>
                </div>`;
    },
    done(version) {
        return `<div class="version-item update-done">
                    <span class="version-main">Restarting...</span>
                    <span class="version-sub">v${version} installed</span>
                </div>`;
    },
    checking(version) {
        return `<div class="version-item">
                    <span class="version-main">v${version}</span>
                    <span class="version-sub">Checking for updates...</span>
                </div>`;
    },
    available(version, state) {
        return `<div class="version-item has-update" data-action="self-update">
                    <span class="version-main">v${state?.latest} available</span>
                    <span class="version-sub">Click to update from v${version}</span>
                </div>`;
    },
    error(version) {
        return `<div class="version-item" data-action="check-update">
                    <span class="version-main">v${version}</span>
                    <span class="version-sub">Update failed. Click to retry</span>
                </div>`;
    },
    idle(version) {
        return `<div class="version-item" data-action="check-update">
                    <span class="version-main">v${version}</span>
                    <span class="version-sub">Check for updates</span>
                </div>`;
    }
};

export function renderVersionFooter(version, state, isDevMode) {
    const status = state?.status || 'idle';
    if (isDevMode) {
        const renderer = DEV_FOOTER_RENDERERS[status] || DEV_FOOTER_RENDERERS.idle;
        return renderer(version, state);
    }

    const renderer = STABLE_FOOTER_RENDERERS[status] || STABLE_FOOTER_RENDERERS.idle;
    return renderer(version, state);
}
