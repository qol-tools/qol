import { prettyLabel } from './auto-config/heuristics.js';
import { renderConfig } from './auto-config/config-renderer.js';
import { configFromForm, getDisplaySections, renderSectionDetail } from './auto-config/normalized-renderer.js';
import { buildFieldPathIndex } from './auto-config/normalized-config.js';
import { installInput } from './auto-config/config-input.js';
import { dissolveIn } from './lib/dissolve.js';
import { setDissolveIn } from './auto-config/variant-renderer.js';

setDissolveIn(dissolveIn);

const SAVE_DEBOUNCE_MS = 400;
const DISSOLVE_DEBOUNCE_MS = 120;
const DISSOLVE_OPTS = {
    renderScale: 2,
    density: 1.0,
    tileSize: 128,
    dissolveRate: 0.2,
    bubbleFade: 0.09,
    maxBatchRate: 0.1,
    origin: 'center',
};

export async function initAutoConfigPage() {
    const state = createInitialState();
    const root = document.getElementById('root');
    try {
        await loadConfig(state);
        if (!state.config) {
            renderErrorPage(root, `${prettyLabel(state.pluginId)} Settings`, 'No configuration found for this plugin.');
            return;
        }
        renderPage(root, state);
        if (state.form) {
            installInput(
                { nav: () => state._navItemsEl, detail: () => state._detailEl },
                {
                    navigate: (delta) => navigateSection(state, delta),
                    focusDetail: () => focusFirstField(state._detailEl),
                    focusNav: () => focusActiveNavItem(state),
                },
            );
        }
    } catch (error) {
        renderErrorPage(root, 'Error', `Failed to load configuration: ${error.message}`);
    }
}

function createInitialState() {
    const pluginId = resolvePluginId();
    const state = {
        pluginId,
        configUrl: `/api/plugins/${pluginId}/config`,
        formUrl: `/api/plugins/${pluginId}/config-form`,
        config: null,
        form: null,
        render: null,
        save: null,
        _activeSectionIndex: 0,
        _sections: [],
        _detailEl: null,
        _navItemsEl: null,
    };
    state.render = () => renderActiveSection(state);
    state.save = createDebouncedSave(state, SAVE_DEBOUNCE_MS);
    return state;
}

function resolvePluginId() {
    return new URLSearchParams(window.location.search).get('plugin')
        || window.location.pathname.split('/').filter(Boolean).pop();
}

async function loadConfig(state) {
    const formResponse = await fetch(state.formUrl);
    if (formResponse.ok) {
        state.form = await formResponse.json();
        state.fieldPaths = buildFieldPathIndex(state.form);
        state.config = configFromForm(state.form);
        return;
    }
    if (formResponse.status !== 404) {
        throw new Error(await readErrorText(formResponse));
    }
    const response = await fetch(state.configUrl);
    if (!response.ok) return;
    state.config = await response.json();
}

function navigateSection(state, delta) {
    const count = state._sections.length;
    const next = (state._activeSectionIndex + delta + count) % count;
    if (next === state._activeSectionIndex) return;
    state._activeSectionIndex = next;
    updateNavActive(state._navItemsEl, next);
    renderActiveSection(state, true);
    focusActiveNavItem(state);
}

function focusActiveNavItem(state) {
    state._navItemsEl?.children[state._activeSectionIndex]?.focus();
}

function focusFirstField(container) {
    if (!container) return;
    const el = container.querySelector('input, select, button');
    if (el) el.focus();
}

function renderPage(root, state) {
    root.replaceChildren();
    if (!state.form) {
        renderFallbackPage(root, state);
        return;
    }
    root.className = 'config-columns';
    state._sections = getDisplaySections(state.form);
    if (state._activeSectionIndex >= state._sections.length) {
        state._activeSectionIndex = 0;
    }
    root.appendChild(createNav(state));
    const detail = document.createElement('div');
    detail.className = 'config-detail';
    state._detailEl = detail;
    root.appendChild(detail);
    renderActiveSection(state);
    requestAnimationFrame(() => focusActiveNavItem(state));
}

function renderFallbackPage(root, state) {
    root.className = 'panel';
    root.appendChild(createHero(state));
    const doc = document.createElement('section');
    doc.className = 'card config-document';
    renderConfig(doc, state.config, state);
    root.append(doc, createSaveStatus());
}

function createNav(state) {
    const nav = document.createElement('nav');
    nav.className = 'config-nav';
    nav.appendChild(createNavHeader(state));
    nav.appendChild(createNavItems(state));
    nav.appendChild(createSaveStatus());
    return nav;
}

function createNavHeader(state) {
    const header = document.createElement('header');
    header.className = 'config-nav-header';
    const h1 = document.createElement('h1');
    h1.textContent = resolveTitle(state);
    header.appendChild(h1);
    const desc = resolveDescription(state);
    if (!desc) return header;
    const p = document.createElement('p');
    p.textContent = desc;
    header.appendChild(p);
    return header;
}

function createNavItems(state) {
    const items = document.createElement('div');
    items.className = 'config-nav-items';
    state._navItemsEl = items;
    state._sections.forEach((section, index) => {
        items.appendChild(createNavItem(section, index, state));
    });
    return items;
}

function createNavItem(section, index, state) {
    const item = document.createElement('button');
    item.type = 'button';
    item.className = 'config-nav-item';
    item.textContent = section.label || prettyLabel(section.id);
    item.tabIndex = index === state._activeSectionIndex ? 0 : -1;
    if (index === state._activeSectionIndex) item.classList.add('active');
    item.addEventListener('click', () => selectSection(index, state));
    return item;
}

function selectSection(index, state) {
    if (state._activeSectionIndex === index) return;
    state._activeSectionIndex = index;
    updateNavActive(state._navItemsEl, index);
    renderActiveSection(state, true);
}

function updateNavActive(items, index) {
    Array.from(items.children).forEach((item, i) => {
        item.classList.toggle('active', i === index);
        item.tabIndex = i === index ? 0 : -1;
    });
}

let _renderTimer = null;

function renderActiveSection(state, withDissolve = false) {
    const detail = state._detailEl;
    if (!detail) return;
    clearTimeout(_renderTimer);
    detail.replaceChildren();
    if (!withDissolve) {
        const section = state._sections[state._activeSectionIndex];
        if (section) renderSectionDetail(detail, section, state.form, state);
        return;
    }
    _renderTimer = setTimeout(() => {
        _renderTimer = null;
        const section = state._sections[state._activeSectionIndex];
        if (section) renderSectionDetail(detail, section, state.form, state);
        dissolveIn(detail, DISSOLVE_OPTS);
    }, DISSOLVE_DEBOUNCE_MS);
}

function createHero(state) {
    const hero = document.createElement('header');
    hero.className = 'hero';
    const heading = document.createElement('h1');
    heading.textContent = resolveTitle(state);
    hero.appendChild(heading);
    const description = resolveDescription(state);
    if (!description) return hero;
    const copy = document.createElement('p');
    copy.textContent = description;
    hero.appendChild(copy);
    return hero;
}

function createSaveStatus() {
    const status = document.createElement('div');
    status.id = 'save-status';
    status.className = 'save-status';
    return status;
}

function resolveTitle(state) {
    if (state.form?.title) return state.form.title;
    return `${prettyLabel(state.pluginId)} Settings`;
}

function resolveDescription(state) {
    return state.form?.description || '';
}

function createDebouncedSave(state, delay) {
    let timer = null;
    return () => {
        clearTimeout(timer);
        timer = setTimeout(() => saveConfig(state), delay);
    };
}

function showStatus(text, isError = false) {
    const el = document.getElementById('save-status');
    if (!el) return;
    el.textContent = text;
    el.classList.toggle('error', isError);
}

async function saveConfig(state) {
    try {
        const response = await fetch(state.configUrl, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(state.config, null, 2),
        });
        if (!response.ok) throw new Error(await readErrorText(response));
        showStatus('Saved');
        setTimeout(() => showStatus(''), 1500);
    } catch (error) {
        console.error('Save failed', error);
        showStatus(formatSaveError(error), true);
        setTimeout(() => showStatus(''), 3000);
    }
}

async function readErrorText(response) {
    const text = await response.text();
    if (text) return text;
    return `Status ${response.status}`;
}

function formatSaveError(error) {
    return (error?.message || 'Failed to save').split('\n')[0] || 'Failed to save';
}

function renderErrorPage(root, title, message) {
    root.replaceChildren();
    const hero = document.createElement('div');
    hero.className = 'hero';
    const heading = document.createElement('h1');
    heading.textContent = title;
    hero.appendChild(heading);
    root.appendChild(hero);
    const card = document.createElement('div');
    card.className = 'card';
    const text = document.createElement('p');
    text.textContent = message;
    card.appendChild(text);
    root.appendChild(card);
}
