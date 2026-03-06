import { prettyLabel } from './heuristics.js';
import { renderConfig } from './config-renderer.js';

export async function initAutoConfigPage() {
    const pluginId = resolvePluginId();
    const state = {
        pluginId,
        configUrl: resolveConfigUrl(pluginId),
        config: null,
    };
    const root = document.getElementById('root');

    try {
        const response = await fetch(state.configUrl);
        if (!response.ok) {
            renderMissingConfig(root, state.pluginId);
            return;
        }
        state.config = await response.json();
    } catch (error) {
        renderLoadError(root, error.message);
        return;
    }

    root.replaceChildren();

    const hero = document.createElement('header');
    hero.className = 'hero';

    const heading = document.createElement('h1');
    heading.textContent = `${prettyLabel(state.pluginId)} Settings`;
    hero.appendChild(heading);
    root.appendChild(hero);

    renderConfig(root, state.config, state);

    const actions = document.createElement('footer');
    actions.className = 'actions';

    const saveButton = document.createElement('button');
    saveButton.id = 'save-btn';
    saveButton.className = 'save';
    saveButton.textContent = 'Save Settings';

    const status = document.createElement('span');
    status.id = 'save-status';

    actions.append(saveButton, status);
    root.appendChild(actions);

    saveButton.addEventListener('click', () => saveConfig(state));
    document.addEventListener('keydown', event => {
        if (event.key === 's' && (event.ctrlKey || event.metaKey)) {
            event.preventDefault();
            saveConfig(state);
        }
    });
}

function resolvePluginId() {
    return new URLSearchParams(window.location.search).get('plugin')
        || window.location.pathname.split('/').filter(Boolean).pop();
}

function resolveConfigUrl(pluginId) {
    return `/api/plugins/${pluginId}/config`;
}

function setStatus(text, isError = false) {
    const el = document.getElementById('save-status');
    if (!el) {
        return;
    }
    el.textContent = text;
    el.classList.toggle('error', isError);
}

async function saveConfig(state) {
    const button = document.getElementById('save-btn');
    if (button) {
        button.disabled = true;
    }
    setStatus('Saving...');

    try {
        const response = await fetch(state.configUrl, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(state.config, null, 2),
        });
        if (!response.ok) {
            throw new Error(`Status ${response.status}`);
        }
        setStatus('Saved');
        setTimeout(() => setStatus(''), 2000);
    } catch (error) {
        console.error('Save failed', error);
        setStatus('Failed to save', true);
        setTimeout(() => setStatus(''), 3000);
    } finally {
        if (button) {
            button.disabled = false;
        }
    }
}

function renderMissingConfig(root, pluginId) {
    root.replaceChildren();

    const hero = document.createElement('div');
    hero.className = 'hero';

    const heading = document.createElement('h1');
    heading.textContent = `${prettyLabel(pluginId)} Settings`;
    hero.appendChild(heading);
    root.appendChild(hero);

    const card = document.createElement('div');
    card.className = 'card';

    const text = document.createElement('p');
    text.textContent = 'No configuration found for this plugin.';
    card.appendChild(text);
    root.appendChild(card);
}

function renderLoadError(root, message) {
    root.replaceChildren();

    const hero = document.createElement('div');
    hero.className = 'hero';

    const heading = document.createElement('h1');
    heading.textContent = 'Error';
    hero.appendChild(heading);
    root.appendChild(hero);

    const card = document.createElement('div');
    card.className = 'card';

    const text = document.createElement('p');
    text.textContent = `Failed to load configuration: ${message}`;
    card.appendChild(text);
    root.appendChild(card);
}
