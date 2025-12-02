import { render as renderSidebar } from './components/sidebar.js';
import * as pluginsView from './views/plugins.js';
import * as storeView from './views/store.js';
import * as hotkeysView from './views/hotkeys.js';

const VIEWS = {
    plugins: pluginsView,
    store: storeView,
    hotkeys: hotkeysView
};

const VIEW_ORDER = ['plugins', 'store', 'hotkeys'];

let activeViewId = 'plugins';
let activeView = null;

function init() {
    const sidebarEl = document.getElementById('sidebar');
    const contentEl = document.getElementById('content');
    
    updateSidebar();
    switchView('plugins');
    
    document.addEventListener('keydown', handleKeydown);
    sidebarEl.addEventListener('click', handleSidebarClick);
}

function updateSidebar() {
    const sidebarEl = document.getElementById('sidebar');
    sidebarEl.innerHTML = renderSidebar(activeViewId);
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
    const item = e.target.closest('.sidebar-item');
    if (!item) return;
    
    const viewId = item.dataset.view;
    if (viewId) {
        switchView(viewId);
    }
}

init();

