import { render as renderSidebar } from './components/sidebar.js';
import { subscribe, onReconnect } from './events.js';
import * as pluginsView from './views/plugins.js';
import * as storeView from './views/store.js';
import * as hotkeysView from './views/hotkeys.js';
import * as taskRunnerView from './features/task-runner/view.js';
import * as devView from './views/dev.js';

const BASE_VIEWS = {
    plugins: pluginsView,
    store: storeView,
    hotkeys: hotkeysView,
    'task-runner': taskRunnerView
};

const BASE_VIEW_ORDER = ['plugins', 'store', 'hotkeys', 'task-runner'];

let VIEWS = { ...BASE_VIEWS };
let VIEW_ORDER = [...BASE_VIEW_ORDER];
let devEnabled = false;
let activeViewId = 'plugins';
let activeView = null;
let appVersion = null;
let updateState = { status: 'checking' };

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
    switchView('plugins');
    if (!devEnabled) {
        checkForUpdate();
    }

    subscribe(handleUpdateEvent);
    onReconnect(() => {
        if (updateState.status === 'done') checkForUpdate();
    });
    document.addEventListener('keydown', handleKeydown);
    sidebarEl.addEventListener('click', handleSidebarClick);
}

function handleUpdateEvent(event) {
    if (devEnabled) {
        if (event.type === 'self_recompile_progress') {
            const percent = Number.isFinite(event.percent) ? event.percent : 0;
            const phase = typeof event.phase === 'string' && event.phase.trim()
                ? event.phase
                : 'Recompiling QoL Tray';
            const label = percent > 0 ? `${phase} ${percent}%` : `${phase}...`;
            updateState = { status: 'compiling', percent, phase };
            const fill = document.querySelector('.progress-fill');
            const sub = document.querySelector('.is-downloading .version-sub');
            if (fill && sub) {
                fill.style.width = `${percent}%`;
                sub.textContent = label;
            } else {
                updateSidebar();
            }
            return;
        }
        if (event.type === 'self_recompile_complete') {
            updateState = { status: 'recompile_done' };
            updateSidebar();
            setTimeout(() => {
                updateState = { status: 'idle' };
                updateSidebar();
            }, 1800);
            return;
        }
        if (event.type === 'self_recompile_failed') {
            updateState = { status: 'error', message: event.message };
            updateSidebar();
            return;
        }
    }

    if (event.type === 'update_progress') {
        updateState = { status: 'downloading', percent: event.percent };
        const fill = document.querySelector('.progress-fill');
        const sub = document.querySelector('.is-downloading .version-sub');
        if (fill && sub) {
            fill.style.width = `${event.percent}%`;
            sub.textContent = event.percent > 0 ? `Downloading ${event.percent}%` : 'Downloading...';
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

function switchView(viewId) {
    if (!VIEWS[viewId]) return;
    
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
    updateState = { status: 'downloading', percent: 0 };
    updateSidebar();
    try {
        await fetch('/api/self-update', { method: 'POST' });
    } catch {
        updateState = { status: 'error' };
        updateSidebar();
    }
}

async function recompileDev() {
    if (!devEnabled || updateState.status === 'compiling') {
        return;
    }

    const item = document.querySelector('.version-item');
    if (item) {
        item.classList.add('update-burst');
        await new Promise(r => setTimeout(r, 400));
    }
    updateState = { status: 'compiling', percent: 0, phase: 'Preparing build' };
    updateSidebar();

    try {
        const response = await fetch('/api/dev/recompile-self', { method: 'POST' });
        if (!response.ok) {
            throw new Error();
        }
    } catch {
        updateState = { status: 'error', message: 'Could not start recompile' };
        updateSidebar();
    }
}

init();
