import { render as renderSidebar, renderVersionFooter } from './components/sidebar.js';
import { subscribe, onReconnect } from './events.js';
import * as pluginsView from './views/plugins.js';
import * as storeView from './views/store.js';
import * as hotkeysView from './views/hotkeys.js';
import * as taskRunnerView from './features/task-runner/view.js';
import * as devView from './views/dev.js';
import { readResponseText } from './api/client.js';
import { clampPercent, formatDownloadingProgress, formatPhaseProgress } from './utils/progress.js';

const BASE_VIEWS = {
    plugins: pluginsView,
    store: storeView,
    hotkeys: hotkeysView,
    'task-runner': taskRunnerView
};

const BASE_VIEW_ORDER = ['plugins', 'store', 'hotkeys', 'task-runner'];
const VIEW_STORAGE_KEY = 'qoltray.activeView';

let VIEWS = { ...BASE_VIEWS };
let VIEW_ORDER = [...BASE_VIEW_ORDER];
let devEnabled = false;
let activeViewId = 'plugins';
let activeView = null;
let appVersion = null;
let updateState = { status: 'checking' };
const devFlows = {
    update: {
        active: false,
        percent: 0,
        done: false,
        error: null,
        clearTimer: null
    },
    recompile: {
        active: false,
        percent: 0,
        phase: 'Preparing build',
        done: false,
        error: null,
        clearTimer: null
    }
};

function parseViewFromHash() {
    const raw = window.location.hash.replace(/^#/, '').trim();
    return raw || null;
}

function canUseView(viewId) {
    return Boolean(viewId) && Object.prototype.hasOwnProperty.call(VIEWS, viewId);
}

function readStoredView() {
    try {
        return window.localStorage.getItem(VIEW_STORAGE_KEY);
    } catch {
        return null;
    }
}

function persistActiveView(viewId, { updateHash = true } = {}) {
    try {
        window.localStorage.setItem(VIEW_STORAGE_KEY, viewId);
    } catch { }

    if (!updateHash) return;
    const targetHash = `#${viewId}`;
    if (window.location.hash !== targetHash) {
        window.history.replaceState(null, '', targetHash);
    }
}

function resolveInitialView() {
    const fromHash = parseViewFromHash();
    if (canUseView(fromHash)) return fromHash;

    const fromStorage = readStoredView();
    if (canUseView(fromStorage)) return fromStorage;

    return 'plugins';
}

async function init() {
    const sidebarEl = document.getElementById('sidebar');

    try {
        const res = await fetch('/api/dev/enabled');
        devEnabled = res.ok && await res.json();
    } catch { devEnabled = false; }

    try {
        const res = await fetch('/api/version');
        if (res.ok) appVersion = await res.text();
    } catch { }

    if (devEnabled) {
        VIEWS = { ...BASE_VIEWS, dev: devView };
        VIEW_ORDER = [...BASE_VIEW_ORDER, 'dev'];
        updateState = { status: 'idle' };
    }

    updateSidebar();
    switchView(resolveInitialView());
    if (!devEnabled) {
        checkForUpdate();
    }

    subscribe(handleUpdateEvent);
    onReconnect(() => {
        if (!devEnabled && updateState.status === 'done') checkForUpdate();
    });
    document.addEventListener('keydown', handleKeydown);
    window.addEventListener('hashchange', handleHashChange);
    sidebarEl.addEventListener('click', handleSidebarClick);
}

function clearDevFlowTimer(flowKey) {
    const flow = devFlows[flowKey];
    if (!flow || !flow.clearTimer) return;
    clearTimeout(flow.clearTimer);
    flow.clearTimer = null;
}

function scheduleDevFlowDoneClear(flowKey, delayMs) {
    clearDevFlowTimer(flowKey);
    const flow = devFlows[flowKey];
    if (!flow) return;
    flow.clearTimer = setTimeout(() => {
        flow.clearTimer = null;
        flow.done = false;
        if (!flow.active && !flow.error) {
            syncDevSidebarState();
        }
    }, delayMs);
}

function resolveDevSidebarState() {
    const recompile = devFlows.recompile;
    const update = devFlows.update;

    if (recompile.error) {
        return { status: 'error', message: recompile.error };
    }
    if (recompile.active) {
        return {
            status: 'compiling',
            percent: recompile.percent,
            phase: recompile.phase || 'Recompiling QoL Tray'
        };
    }

    if (update.error) {
        return { status: 'error', message: update.error };
    }
    if (update.active) {
        return {
            status: 'downloading',
            percent: update.percent
        };
    }

    if (recompile.done) {
        return { status: 'recompile_done' };
    }
    if (update.done) {
        return { status: 'done' };
    }

    return { status: 'idle' };
}

function syncDevSidebarState() {
    const nextState = resolveDevSidebarState();
    const previousStatus = updateState?.status;
    updateState = nextState;

    const canPatchInPlace =
        previousStatus === nextState.status &&
        (nextState.status === 'compiling' || nextState.status === 'downloading');

    if (canPatchInPlace && patchSidebarProgressInPlace(nextState)) {
        return;
    }

    updateSidebarVersionOnly();
}

function handleUpdateEvent(event) {
    if (devEnabled) {
        if (event.type === 'self_recompile_progress') {
            clearDevFlowTimer('recompile');
            const percent = clampPercent(event.percent);
            const phase = typeof event.phase === 'string' && event.phase.trim() ? event.phase : 'Recompiling QoL Tray';
            devFlows.recompile.active = true;
            devFlows.recompile.percent = percent;
            devFlows.recompile.phase = phase;
            devFlows.recompile.done = false;
            devFlows.recompile.error = null;
            syncDevSidebarState();
            return;
        }
        if (event.type === 'self_recompile_complete') {
            clearDevFlowTimer('recompile');
            devFlows.recompile.active = false;
            devFlows.recompile.percent = 100;
            devFlows.recompile.done = true;
            devFlows.recompile.error = null;
            syncDevSidebarState();
            scheduleDevFlowDoneClear('recompile', 1800);
            return;
        }
        if (event.type === 'self_recompile_failed') {
            clearDevFlowTimer('recompile');
            devFlows.recompile.active = false;
            devFlows.recompile.done = false;
            devFlows.recompile.error = event.message || 'Recompile failed';
            syncDevSidebarState();
            return;
        }

        if (event.type === 'update_progress') {
            clearDevFlowTimer('update');
            const percent = clampPercent(event.percent);
            devFlows.update.active = true;
            devFlows.update.percent = percent;
            devFlows.update.done = false;
            devFlows.update.error = null;
            syncDevSidebarState();
            return;
        } else if (event.type === 'update_complete') {
            clearDevFlowTimer('update');
            devFlows.update.active = false;
            devFlows.update.percent = 100;
            devFlows.update.done = true;
            devFlows.update.error = null;
            syncDevSidebarState();
            scheduleDevFlowDoneClear('update', 2000);
            return;
        } else if (event.type === 'update_failed') {
            clearDevFlowTimer('update');
            devFlows.update.active = false;
            devFlows.update.done = false;
            devFlows.update.error = event.message || 'Update failed';
            syncDevSidebarState();
            return;
        }

        return;
    }

    if (event.type === 'update_progress') {
        const percent = clampPercent(event.percent);
        updateState = { status: 'downloading', percent };
        const fill = document.querySelector('.progress-fill');
        const sub = document.querySelector('.is-downloading .version-sub');
        if (fill && sub) {
            fill.style.width = `${percent}%`;
            sub.textContent = formatDownloadingProgress(percent);
        } else {
            updateSidebar();
        }
        return;
    } else if (event.type === 'update_complete') {
        updateState = { status: 'done' };
        updateSidebar();
        if (devEnabled) {
            setTimeout(() => {
                updateState = { status: 'idle' };
                updateSidebar();
            }, 2000);
        } else {
            setTimeout(() => {
                if (updateState.status === 'done') checkForUpdate();
            }, 30000);
        }
    } else if (event.type === 'update_failed') {
        updateState = { status: 'error' };
        updateSidebar();
    }
}

function updateSidebar() {
    const sidebarEl = document.getElementById('sidebar');
    sidebarEl.innerHTML = renderSidebar(activeViewId, VIEW_ORDER, appVersion, updateState, devEnabled);
}

function updateSidebarVersionOnly() {
    if (!appVersion) return;

    const sidebarEl = document.getElementById('sidebar');
    const versionRoot = sidebarEl?.querySelector('.sidebar-version');
    if (!versionRoot) {
        updateSidebar();
        return;
    }

    versionRoot.innerHTML = renderVersionFooter(appVersion, updateState, devEnabled);
}

function sidebarProgressText(state) {
    if (state.status === 'compiling') {
        return formatPhaseProgress(state.phase, state.percent, 'Recompiling QoL Tray');
    }

    if (state.status === 'downloading') {
        return formatDownloadingProgress(state.percent);
    }

    return '';
}

function patchSidebarProgressInPlace(state) {
    const versionItem = document.querySelector('#sidebar .sidebar-version .version-item');
    if (!versionItem) return false;

    const fill = versionItem.querySelector('.progress-fill');
    const sub = versionItem.querySelector('.version-sub');
    if (!fill || !sub) return false;

    const percent = clampPercent(state.percent);
    fill.style.width = `${percent}%`;
    sub.textContent = sidebarProgressText({ ...state, percent });
    return true;
}

function switchView(viewId, options = {}) {
    if (!VIEWS[viewId]) return;

    if (activeViewId === viewId && activeView) {
        persistActiveView(viewId, { updateHash: options.updateHash !== false });
        return;
    }

    if (activeView && activeView.onBlur) {
        activeView.onBlur();
    }

    activeViewId = viewId;
    activeView = VIEWS[viewId];

    updateSidebar();

    const contentEl = document.getElementById('content');
    contentEl.innerHTML = '';
    activeView.render(contentEl);

    if (activeView.onFocus) {
        activeView.onFocus();
    }

    persistActiveView(viewId, { updateHash: options.updateHash !== false });
}

function handleHashChange() {
    const viewId = parseViewFromHash();
    if (!canUseView(viewId) || viewId === activeViewId) return;
    switchView(viewId, { updateHash: false });
}

function handleKeydown(e) {
    if (activeView?.isBlocking?.()) {
        if (activeView.handleKey) {
            activeView.handleKey(e);
        }
        return;
    }
    
    if (e.key === 'Tab' && !e.shiftKey) {
        e.preventDefault();
        const currentIndex = VIEW_ORDER.indexOf(activeViewId);
        const nextIndex = (currentIndex + 1) % VIEW_ORDER.length;
        switchView(VIEW_ORDER[nextIndex]);
        return;
    }
    
    if (e.key === 'Tab' && e.shiftKey) {
        e.preventDefault();
        const currentIndex = VIEW_ORDER.indexOf(activeViewId);
        const prevIndex = (currentIndex - 1 + VIEW_ORDER.length) % VIEW_ORDER.length;
        switchView(VIEW_ORDER[prevIndex]);
        return;
    }
    
    if (activeView && activeView.handleKey) {
        activeView.handleKey(e);
    }
}

function handleSidebarClick(e) {
    const updateBtn = e.target.closest('[data-action]');
    if (updateBtn) {
        const action = updateBtn.dataset.action;
        if (action === 'check-update') checkForUpdate();
        if (action === 'self-update') triggerSelfUpdate();
        if (action === 'dev-recompile') recompileDev();
        return;
    }

    const item = e.target.closest('.sidebar-item');
    if (!item) return;
    
    const viewId = item.dataset.view;
    if (viewId) {
        switchView(viewId);
    }
}

async function checkForUpdate() {
    updateState = { status: 'checking' };
    updateSidebar();
    const minDelay = new Promise(r => setTimeout(r, 800));
    let result;
    try {
        const res = await fetch('/api/check-update');
        if (!res.ok) throw new Error();
        result = await res.json();
    } catch {
        result = null;
    }
    await minDelay;
    updateState = result
        ? (result.available ? { status: 'available', latest: result.latest } : { status: 'up-to-date' })
        : { status: 'error' };
    updateSidebar();
}

async function triggerSelfUpdate() {
    const item = document.querySelector('.version-item');
    if (item) {
        item.classList.add('update-burst');
        await new Promise(r => setTimeout(r, 400));
    }
    if (devEnabled) {
        clearDevFlowTimer('update');
        devFlows.update.active = true;
        devFlows.update.percent = 0;
        devFlows.update.done = false;
        devFlows.update.error = null;
        syncDevSidebarState();
    } else {
        updateState = { status: 'downloading', percent: 0 };
        updateSidebar();
    }
    try {
        await fetch('/api/self-update', { method: 'POST' });
    } catch {
        if (devEnabled) {
            clearDevFlowTimer('update');
            devFlows.update.active = false;
            devFlows.update.done = false;
            devFlows.update.error = 'Update failed';
            syncDevSidebarState();
        } else {
            updateState = { status: 'error' };
            updateSidebar();
        }
    }
}

async function recompileDev() {
    if (!devEnabled || devFlows.recompile.active || devFlows.update.active) {
        return;
    }

    const item = document.querySelector('.version-item');
    if (item) {
        item.classList.add('update-burst');
        await new Promise(r => setTimeout(r, 400));
    }
    clearDevFlowTimer('recompile');
    devFlows.recompile.active = true;
    devFlows.recompile.percent = 0;
    devFlows.recompile.phase = 'Preparing build';
    devFlows.recompile.done = false;
    devFlows.recompile.error = null;
    syncDevSidebarState();

    try {
        const response = await fetch('/api/dev/recompile-self', { method: 'POST' });
        if (!response.ok) {
            const body = await readResponseText(response);
            if (response.status === 404) {
                throw new Error('Connected daemon is older than this UI. Stop it and launch the current checkout.');
            }
            if (response.status === 409) {
                throw new Error('Recompile already in progress');
            }
            throw new Error(body || `Could not start recompile (${response.status})`);
        }
    } catch (error) {
        const message = error?.message || 'Could not start recompile';
        clearDevFlowTimer('recompile');
        devFlows.recompile.active = false;
        devFlows.recompile.done = false;
        devFlows.recompile.error = message;
        syncDevSidebarState();
    }
}

init();
