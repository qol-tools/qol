const PLUGIN_ID = window.location.pathname.split('/')[2];
const CONFIG_URL = `/api/plugins/${PLUGIN_ID}/config`;

const DEFAULT_CONFIG = {
    enabled: true,
    excluded_apps: [],
    key_rules: [],
    mouse_rules: [],
    scroll_rules: []
};

const VALID_MODS = new Set(['ctrl', 'shift', 'alt', 'cmd']);

let config = { ...DEFAULT_CONFIG };

const elements = {
    saveBtn: document.getElementById('save-btn'),
    saveStatus: document.getElementById('save-status'),
    excludedAppsList: document.getElementById('excluded-apps-list'),
    keyRulesList: document.getElementById('key-rules-list'),
    mouseRulesList: document.getElementById('mouse-rules-list'),
    scrollRulesList: document.getElementById('scroll-rules-list'),
};

function normalizeMods(mods) {
    if (!Array.isArray(mods)) return [];
    return mods.filter(m => VALID_MODS.has(m));
}

function normalizeConfig(raw) {
    const enabled = typeof raw?.enabled === 'boolean' ? raw.enabled : true;

    const excludedApps = Array.isArray(raw?.excluded_apps)
        ? raw.excluded_apps.filter(a => typeof a === 'string' && a.length > 0)
        : [];

    const keyRules = Array.isArray(raw?.key_rules)
        ? raw.key_rules.filter(r => (r?.keys && r.keys.length > 0) || (r?.from_key && r?.to_key)).map(r => {
            if (r.keys) {
                return {
                    from_mods: normalizeMods(r.from_mods),
                    to_mods: normalizeMods(r.to_mods),
                    keys: r.keys.filter(k => typeof k === 'string' && k.length > 0),
                };
            }
            return {
                from_mods: normalizeMods(r.from_mods),
                from_key: String(r.from_key),
                to_mods: normalizeMods(r.to_mods),
                to_key: String(r.to_key),
            };
        })
        : [];

    const mouseRules = Array.isArray(raw?.mouse_rules)
        ? raw.mouse_rules.filter(r => r?.button).map(r => ({
            from_mods: normalizeMods(r.from_mods),
            button: String(r.button),
            to_mods: normalizeMods(r.to_mods),
        }))
        : [];

    const scrollRules = Array.isArray(raw?.scroll_rules)
        ? raw.scroll_rules.map(r => ({
            from_mods: normalizeMods(r?.from_mods),
            to_mods: normalizeMods(r?.to_mods),
        }))
        : [];

    return { enabled, excluded_apps: excludedApps, key_rules: keyRules, mouse_rules: mouseRules, scroll_rules: scrollRules };
}

function renderModChips(mods) {
    return mods.map(m => `<span class="mod-chip-static">${m}</span>`).join(' ');
}

function renderExcludedApps() {
    const list = elements.excludedAppsList;
    if (config.excluded_apps.length === 0) {
        list.innerHTML = '<div class="empty-state">No excluded apps. All apps will have remapping active.</div>';
        return;
    }
    list.innerHTML = config.excluded_apps.map((app, i) =>
        `<div class="item-row">
            <span>${app}</span>
            <button class="btn-remove" data-type="app" data-index="${i}">&times;</button>
        </div>`
    ).join('');
}

function renderKeyChips(keys) {
    return keys.map(k => `<span class="key-chip">${k}</span>`).join(' ');
}

function renderKeyRules() {
    const list = elements.keyRulesList;
    if (config.key_rules.length === 0) {
        list.innerHTML = '<div class="empty-state">No key rules defined.</div>';
        return;
    }
    list.innerHTML = config.key_rules.map((rule, i) => {
        if (rule.keys) {
            return `<div class="rule-row batch-rule">
                <div class="rule-side">${renderModChips(rule.from_mods)} ${renderKeyChips(rule.keys)}</div>
                <span class="arrow">&rarr;</span>
                <div class="rule-side">${renderModChips(rule.to_mods)} <span class="key-label-hint">same key</span></div>
                <button class="btn-remove" data-type="key" data-index="${i}">&times;</button>
            </div>`;
        }
        return `<div class="rule-row">
            <div class="rule-side">${renderModChips(rule.from_mods)} <span class="key-label">${rule.from_key}</span></div>
            <span class="arrow">&rarr;</span>
            <div class="rule-side">${renderModChips(rule.to_mods)} <span class="key-label">${rule.to_key}</span></div>
            <button class="btn-remove" data-type="key" data-index="${i}">&times;</button>
        </div>`;
    }).join('');
}

function renderMouseRules() {
    const list = elements.mouseRulesList;
    if (config.mouse_rules.length === 0) {
        list.innerHTML = '<div class="empty-state">No mouse rules defined.</div>';
        return;
    }
    list.innerHTML = config.mouse_rules.map((rule, i) =>
        `<div class="rule-row">
            <div class="rule-side">${renderModChips(rule.from_mods)} <span class="key-label">${rule.button} click</span></div>
            <span class="arrow">&rarr;</span>
            <div class="rule-side">${renderModChips(rule.to_mods)}</div>
            <button class="btn-remove" data-type="mouse" data-index="${i}">&times;</button>
        </div>`
    ).join('');
}

function renderScrollRules() {
    const list = elements.scrollRulesList;
    if (config.scroll_rules.length === 0) {
        list.innerHTML = '<div class="empty-state">No scroll rules defined.</div>';
        return;
    }
    list.innerHTML = config.scroll_rules.map((rule, i) =>
        `<div class="rule-row">
            <div class="rule-side">${renderModChips(rule.from_mods)} <span class="key-label">scroll</span></div>
            <span class="arrow">&rarr;</span>
            <div class="rule-side">${renderModChips(rule.to_mods)} <span class="key-label">scroll</span></div>
            <button class="btn-remove" data-type="scroll" data-index="${i}">&times;</button>
        </div>`
    ).join('');
}

function renderAll() {
    const toggle = document.getElementById('enabled-toggle');
    if (toggle) toggle.checked = config.enabled;
    renderExcludedApps();
    renderKeyRules();
    renderMouseRules();
    renderScrollRules();
}

function getActiveMods(targetName) {
    const container = document.querySelector(`.mod-toggles[data-target="${targetName}"]`);
    if (!container) return [];
    return Array.from(container.querySelectorAll('.mod-chip.active')).map(b => b.dataset.mod);
}

function clearModToggles(targetName) {
    const container = document.querySelector(`.mod-toggles[data-target="${targetName}"]`);
    if (!container) return;
    container.querySelectorAll('.mod-chip').forEach(b => b.classList.remove('active'));
}

document.querySelectorAll('.mod-toggles .mod-chip').forEach(btn => {
    btn.addEventListener('click', (e) => {
        e.preventDefault();
        btn.classList.toggle('active');
    });
});

document.addEventListener('click', (e) => {
    const btn = e.target.closest('.btn-remove');
    if (!btn) return;

    const type = btn.dataset.type;
    const index = parseInt(btn.dataset.index, 10);

    if (type === 'app') {
        config.excluded_apps.splice(index, 1);
        renderExcludedApps();
    } else if (type === 'key') {
        config.key_rules.splice(index, 1);
        renderKeyRules();
    } else if (type === 'mouse') {
        config.mouse_rules.splice(index, 1);
        renderMouseRules();
    } else if (type === 'scroll') {
        config.scroll_rules.splice(index, 1);
        renderScrollRules();
    }
});

document.getElementById('add-app-btn').addEventListener('click', () => {
    const input = document.getElementById('new-app-input');
    const value = input.value.trim();
    if (!value) return;
    if (config.excluded_apps.includes(value)) return;
    config.excluded_apps.push(value);
    input.value = '';
    renderExcludedApps();
});

document.getElementById('new-app-input').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
        e.preventDefault();
        document.getElementById('add-app-btn').click();
    }
});

document.querySelectorAll('.add-rule-tabs .tab-btn').forEach(btn => {
    btn.addEventListener('click', () => {
        const tab = btn.dataset.tab;
        btn.closest('.card').querySelectorAll('.tab-btn').forEach(b => b.classList.toggle('active', b.dataset.tab === tab));
        btn.closest('.card').querySelectorAll('[data-tab-content]').forEach(el => {
            el.classList.toggle('hidden', el.dataset.tabContent !== tab);
        });
    });
});

document.getElementById('add-key-batch-btn').addEventListener('click', () => {
    const fromMods = getActiveMods('new-key-batch-from');
    const toMods = getActiveMods('new-key-batch-to');
    const keysInput = document.getElementById('new-key-batch-keys').value.trim();

    const keys = keysInput.split(',').map(k => k.trim().toLowerCase()).filter(k => k.length > 0);
    if (keys.length === 0) return;

    config.key_rules.push({ from_mods: fromMods, to_mods: toMods, keys });
    document.getElementById('new-key-batch-keys').value = '';
    clearModToggles('new-key-batch-from');
    clearModToggles('new-key-batch-to');
    renderKeyRules();
});

document.getElementById('add-key-rule-btn').addEventListener('click', () => {
    const fromMods = getActiveMods('new-key-from');
    const fromKey = document.getElementById('new-key-from-key').value.trim().toLowerCase();
    const toMods = getActiveMods('new-key-to');
    const toKey = document.getElementById('new-key-to-key').value.trim().toLowerCase();

    if (!fromKey || !toKey) return;

    config.key_rules.push({ from_mods: fromMods, from_key: fromKey, to_mods: toMods, to_key: toKey });
    document.getElementById('new-key-from-key').value = '';
    document.getElementById('new-key-to-key').value = '';
    clearModToggles('new-key-from');
    clearModToggles('new-key-to');
    renderKeyRules();
});

document.getElementById('add-mouse-rule-btn').addEventListener('click', () => {
    const fromMods = getActiveMods('new-mouse-from');
    const button = document.getElementById('new-mouse-button').value;
    const toMods = getActiveMods('new-mouse-to');

    config.mouse_rules.push({ from_mods: fromMods, button, to_mods: toMods });
    clearModToggles('new-mouse-from');
    clearModToggles('new-mouse-to');
    renderMouseRules();
});

document.getElementById('add-scroll-rule-btn').addEventListener('click', () => {
    const fromMods = getActiveMods('new-scroll-from');
    const toMods = getActiveMods('new-scroll-to');

    config.scroll_rules.push({ from_mods: fromMods, to_mods: toMods });
    clearModToggles('new-scroll-from');
    clearModToggles('new-scroll-to');
    renderScrollRules();
});

function setStatus(text, isError = false) {
    elements.saveStatus.textContent = text;
    elements.saveStatus.classList.toggle('error', isError);
}

async function loadConfig() {
    try {
        const response = await fetch(CONFIG_URL);
        if (response.ok) {
            config = normalizeConfig(await response.json());
        }
    } catch (error) {
        console.warn('Could not load keyremap config, using defaults', error);
    }
    renderAll();
}

function collectConfig() {
    const toggle = document.getElementById('enabled-toggle');
    config.enabled = toggle ? toggle.checked : true;
    return config;
}

async function saveConfig() {
    elements.saveBtn.disabled = true;
    setStatus('Saving...');

    try {
        const response = await fetch(CONFIG_URL, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(collectConfig(), null, 2)
        });

        if (!response.ok) {
            throw new Error(`Save failed with status ${response.status}`);
        }

        setStatus('Saved');
        setTimeout(() => setStatus(''), 2000);
    } catch (error) {
        console.error('Failed to save keyremap config', error);
        setStatus('Failed to save', true);
        setTimeout(() => setStatus(''), 3000);
    } finally {
        elements.saveBtn.disabled = false;
    }
}

elements.saveBtn.addEventListener('click', saveConfig);

document.addEventListener('keydown', (event) => {
    if (event.key === 's' && (event.ctrlKey || event.metaKey)) {
        event.preventDefault();
        saveConfig();
    }
});

loadConfig();
