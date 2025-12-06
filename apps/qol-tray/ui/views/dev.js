export const id = 'dev';

const state = {
    reloading: false,
    lastReload: null,
    error: null
};

let container = null;

export function render(containerEl) {
    container = containerEl;
    updateView();
}

function updateView() {
    container.innerHTML = `
        <div class="view-container">
            <header>
                <h1>Developer</h1>
                <p>Tools for plugin development and debugging</p>
            </header>
            <div class="dev-actions">
                <div class="dev-card ${state.reloading ? 'loading' : ''}" data-action="reload">
                    <div class="dev-card-icon">&#x21bb;</div>
                    <div class="dev-card-content">
                        <h3>Reload All Plugins</h3>
                        <p>Stop all plugin daemons and restart them. Use after rebuilding plugins.</p>
                        ${state.lastReload ? `<span class="last-action">Last: ${state.lastReload}</span>` : ''}
                        ${state.error ? `<span class="error-msg">${state.error}</span>` : ''}
                    </div>
                    <div class="dev-card-hint"><kbd>r</kbd></div>
                </div>
            </div>
            <footer class="help">
                r reload plugins
            </footer>
        </div>
    `;

    container.querySelector('.dev-actions').addEventListener('click', handleClick);
}

function handleClick(e) {
    const card = e.target.closest('.dev-card');
    if (!card) return;

    const action = card.dataset.action;
    if (action === 'reload') {
        reloadPlugins();
    }
}

async function reloadPlugins() {
    if (state.reloading) return;

    state.reloading = true;
    state.error = null;
    updateView();

    try {
        const res = await fetch('/api/dev/reload', { method: 'POST' });
        if (res.ok) {
            state.lastReload = new Date().toLocaleTimeString();
        } else {
            const text = await res.text();
            state.error = text || 'Reload failed';
        }
    } catch (err) {
        state.error = err.message;
    } finally {
        state.reloading = false;
        updateView();
    }
}

export function handleKey(e) {
    if (e.key === 'r' || e.key === 'R') {
        e.preventDefault();
        reloadPlugins();
    }
}

export function onFocus() {}
export function onBlur() {}
