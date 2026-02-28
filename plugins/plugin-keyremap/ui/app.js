const PLUGIN_ID = window.location.pathname.split('/')[2];
const CONFIG_URL = `/api/plugins/${PLUGIN_ID}/config`;

const DEFAULT_CONFIG = {
    enabled: true,
    excluded_apps: [],
    char_rules: [],
    key_rules: [],
    mouse_rules: [],
    scroll_rules: []
};

const VALID_MODS = new Set(['ctrl', 'shift', 'alt', 'cmd', 'ralt', 'altgr']);

let config = { ...DEFAULT_CONFIG };
let pendingWarnings = false;

// App picker state
let allApps = null; // cached from /api/apps
let appsLoading = false;

const elements = {
    saveBtn: document.getElementById('save-btn'),
    saveStatus: document.getElementById('save-status'),
    excludedAppsList: document.getElementById('excluded-apps-list'),
    charRulesList: document.getElementById('char-rules-list'),
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

    const charRules = Array.isArray(raw?.char_rules)
        ? raw.char_rules.filter(r => r?.from_key && r?.to_char).map(r => ({
            from_mods: normalizeMods(r.from_mods),
            from_key: String(r.from_key),
            to_char: String(r.to_char),
            global: !!r.global,
        }))
        : [];

    const keyRules = Array.isArray(raw?.key_rules)
        ? raw.key_rules.filter(r => (r?.keys && r.keys.length > 0) || (r?.from_key && r?.to_key)).map(r => {
            if (r.keys) {
                return {
                    from_mods: normalizeMods(r.from_mods),
                    to_mods: normalizeMods(r.to_mods),
                    keys: r.keys.filter(k => typeof k === 'string' && k.length > 0),
                    global: !!r.global,
                };
            }
            return {
                from_mods: normalizeMods(r.from_mods),
                from_key: String(r.from_key),
                to_mods: normalizeMods(r.to_mods),
                to_key: String(r.to_key),
                global: !!r.global,
            };
        })
        : [];

    const mouseRules = Array.isArray(raw?.mouse_rules)
        ? raw.mouse_rules.filter(r => r?.button).map(r => ({
            from_mods: normalizeMods(r.from_mods),
            button: String(r.button),
            to_mods: normalizeMods(r.to_mods),
            global: !!r.global,
        }))
        : [];

    const scrollRules = Array.isArray(raw?.scroll_rules)
        ? raw.scroll_rules.map(r => ({
            from_mods: normalizeMods(r?.from_mods),
            to_mods: normalizeMods(r?.to_mods),
            global: !!r.global,
        }))
        : [];

    return { enabled, excluded_apps: excludedApps, char_rules: charRules, key_rules: keyRules, mouse_rules: mouseRules, scroll_rules: scrollRules };
}

function renderModChips(mods) {
    return mods.map(m => `<span class="mod-chip-static">${m}</span>`).join(' ');
}

function appNameFor(bundleId) {
    if (!allApps) return null;
    const entry = allApps.find(a => a.bundle_id === bundleId);
    return entry ? entry.name : null;
}

function escHtml(s) {
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function renderExcludedApps() {
    const list = elements.excludedAppsList;
    if (config.excluded_apps.length === 0) {
        list.innerHTML = '<div class="empty-state">No excluded apps. All apps will have remapping active.</div>';
        return;
    }
    list.innerHTML = config.excluded_apps.map((app, i) => {
        const name = appNameFor(app);
        const label = name
            ? `<span class="app-name">${escHtml(name)}</span><span class="app-separator">&mdash;</span><span class="app-bid">${escHtml(app)}</span>`
            : `<span>${escHtml(app)}</span>`;
        return `<div class="item-row">
            <img class="app-icon" src="/api/icon/${encodeURIComponent(app)}" width="20" height="20" onerror="this.style.display='none'">
            ${label}
            <button class="btn-remove" data-type="app" data-index="${i}">&times;</button>
        </div>`;
    }).join('');
}

function renderKeyChips(keys) {
    return keys.map(k => `<span class="key-chip">${k}</span>`).join(' ');
}

function renderCharRules() {
    const list = elements.charRulesList;
    if (config.char_rules.length === 0) {
        list.innerHTML = '<div class="empty-state">No char rules defined.</div>';
        return;
    }
    list.innerHTML = config.char_rules.map((rule, i) =>
        `<div class="rule-row">
            <div class="rule-side">${renderModChips(rule.from_mods)} <span class="key-label">${rule.from_key}</span></div>
            <span class="arrow">&rarr;</span>
            <div class="rule-side"><span class="key-label char-output">${rule.to_char}</span></div>
            ${rule.global ? '<span class="global-badge">global</span>' : ''}
            <button class="btn-remove" data-type="char" data-index="${i}">&times;</button>
        </div>`
    ).join('');
}

function renderKeyRules() {
    const list = elements.keyRulesList;
    if (config.key_rules.length === 0) {
        list.innerHTML = '<div class="empty-state">No key rules defined.</div>';
        return;
    }
    list.innerHTML = config.key_rules.map((rule, i) => {
        const globalBadge = rule.global ? '<span class="global-badge">global</span>' : '';
        if (rule.keys) {
            return `<div class="rule-row batch-rule">
                <div class="rule-side">${renderModChips(rule.from_mods)} ${renderKeyChips(rule.keys)}</div>
                <span class="arrow">&rarr;</span>
                <div class="rule-side">${renderModChips(rule.to_mods)} <span class="key-label-hint">same key</span></div>
                ${globalBadge}
                <button class="btn-remove" data-type="key" data-index="${i}">&times;</button>
            </div>`;
        }
        return `<div class="rule-row">
            <div class="rule-side">${renderModChips(rule.from_mods)} <span class="key-label">${rule.from_key}</span></div>
            <span class="arrow">&rarr;</span>
            <div class="rule-side">${renderModChips(rule.to_mods)} <span class="key-label">${rule.to_key}</span></div>
            ${globalBadge}
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
            ${rule.global ? '<span class="global-badge">global</span>' : ''}
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
            ${rule.global ? '<span class="global-badge">global</span>' : ''}
            <button class="btn-remove" data-type="scroll" data-index="${i}">&times;</button>
        </div>`
    ).join('');
}

function renderAll() {
    const toggle = document.getElementById('enabled-toggle');
    if (toggle) toggle.checked = config.enabled;
    renderExcludedApps();
    renderCharRules();
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

function clearSaveWarnings() {
    pendingWarnings = false;
    const el = document.getElementById('save-warnings');
    if (el) el.innerHTML = '';
}

document.addEventListener('click', (e) => {
    const btn = e.target.closest('.btn-remove');
    if (!btn) return;

    const type = btn.dataset.type;
    const index = parseInt(btn.dataset.index, 10);

    clearSaveWarnings();

    if (type === 'app') {
        config.excluded_apps.splice(index, 1);
        renderExcludedApps();
    } else if (type === 'char') {
        config.char_rules.splice(index, 1);
        renderCharRules();
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

function addExcludedApp(bundleId) {
    if (!bundleId || config.excluded_apps.includes(bundleId)) return;
    config.excluded_apps.push(bundleId);
    renderExcludedApps();
    renderPickerDropdown();
}

document.getElementById('add-app-btn').addEventListener('click', () => {
    const input = document.getElementById('new-app-input');
    const value = input.value.trim();
    if (!value) return;
    addExcludedApp(value);
    input.value = '';
    hidePickerDropdown();
});

document.getElementById('new-app-input').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
        e.preventDefault();
        document.getElementById('add-app-btn').click();
    }
    if (e.key === 'Escape') {
        hidePickerDropdown();
        e.target.blur();
    }
});

// --- App picker dropdown ---

const pickerDropdown = document.getElementById('app-picker-dropdown');
const appInput = document.getElementById('new-app-input');

async function fetchApps() {
    if (allApps !== null || appsLoading) return;
    appsLoading = true;
    pickerDropdown.innerHTML = '<div class="app-picker-loading">Loading apps...</div>';
    pickerDropdown.classList.remove('hidden');
    try {
        const res = await fetch('/api/apps');
        if (res.ok) {
            allApps = await res.json();
            renderExcludedApps(); // re-render with names
        } else {
            allApps = [];
        }
    } catch {
        allApps = [];
    } finally {
        appsLoading = false;
        renderPickerDropdown();
    }
}

function renderPickerDropdown() {
    if (!allApps || allApps.length === 0) {
        pickerDropdown.innerHTML = '<div class="app-picker-empty">No apps found</div>';
        return;
    }

    const query = appInput.value.trim().toLowerCase();
    const filtered = allApps.filter(app => {
        if (!query) return true;
        return app.name.toLowerCase().includes(query)
            || app.bundle_id.toLowerCase().includes(query);
    });

    if (filtered.length === 0) {
        pickerDropdown.innerHTML = '<div class="app-picker-empty">No matches</div>';
        return;
    }

    pickerDropdown.innerHTML = filtered.map(app => {
        const excluded = config.excluded_apps.includes(app.bundle_id);
        return `<div class="app-picker-item${excluded ? ' disabled' : ''}" data-bid="${escHtml(app.bundle_id)}">
            <img src="/api/icon/${encodeURIComponent(app.bundle_id)}" width="24" height="24" onerror="this.style.display='none'">
            <div class="app-picker-info">
                <span class="app-picker-name">${escHtml(app.name)}</span>
                <span class="app-picker-bid">${escHtml(app.bundle_id)}</span>
            </div>
        </div>`;
    }).join('');
}

function showPickerDropdown() {
    pickerDropdown.classList.remove('hidden');
    fetchApps();
}

function hidePickerDropdown() {
    pickerDropdown.classList.add('hidden');
}

appInput.addEventListener('focus', showPickerDropdown);
appInput.addEventListener('input', renderPickerDropdown);

pickerDropdown.addEventListener('click', (e) => {
    const item = e.target.closest('.app-picker-item');
    if (!item || item.classList.contains('disabled')) return;
    const bid = item.dataset.bid;
    appInput.value = '';
    addExcludedApp(bid);
});

document.addEventListener('click', (e) => {
    if (!e.target.closest('.app-picker-container')) {
        hidePickerDropdown();
    }
});

document.getElementById('add-char-rule-btn').addEventListener('click', () => {
    const fromMods = getActiveMods('new-char-from');
    const fromKey = document.getElementById('new-char-from-key').value.trim().toLowerCase();
    const toChar = document.getElementById('new-char-to-char').value;
    const global = document.getElementById('new-char-global').checked;

    if (!fromKey || !toChar) return;

    config.char_rules.push({ from_mods: fromMods, from_key: fromKey, to_char: toChar, global });
    document.getElementById('new-char-from-key').value = '';
    document.getElementById('new-char-to-char').value = '';
    document.getElementById('new-char-global').checked = false;
    clearModToggles('new-char-from');
    renderCharRules();
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

    const global = document.getElementById('new-key-batch-global').checked;
    config.key_rules.push({ from_mods: fromMods, to_mods: toMods, keys, global });
    document.getElementById('new-key-batch-keys').value = '';
    document.getElementById('new-key-batch-global').checked = false;
    clearModToggles('new-key-batch-from');
    clearModToggles('new-key-batch-to');
    clearSaveWarnings();
    renderKeyRules();
});

document.getElementById('add-key-rule-btn').addEventListener('click', () => {
    const fromMods = getActiveMods('new-key-from');
    const fromKey = document.getElementById('new-key-from-key').value.trim().toLowerCase();
    const toMods = getActiveMods('new-key-to');
    const toKey = document.getElementById('new-key-to-key').value.trim().toLowerCase();

    if (!fromKey || !toKey) return;

    const global = document.getElementById('new-key-single-global').checked;
    clearSaveWarnings();
    config.key_rules.push({ from_mods: fromMods, from_key: fromKey, to_mods: toMods, to_key: toKey, global });
    document.getElementById('new-key-from-key').value = '';
    document.getElementById('new-key-to-key').value = '';
    document.getElementById('new-key-single-global').checked = false;
    clearModToggles('new-key-from');
    clearModToggles('new-key-to');
    renderKeyRules();
});

document.getElementById('add-mouse-rule-btn').addEventListener('click', () => {
    const fromMods = getActiveMods('new-mouse-from');
    const button = document.getElementById('new-mouse-button').value;
    const toMods = getActiveMods('new-mouse-to');
    const global = document.getElementById('new-mouse-global').checked;

    config.mouse_rules.push({ from_mods: fromMods, button, to_mods: toMods, global });
    document.getElementById('new-mouse-global').checked = false;
    clearModToggles('new-mouse-from');
    clearModToggles('new-mouse-to');
    renderMouseRules();
});

document.getElementById('add-scroll-rule-btn').addEventListener('click', () => {
    const fromMods = getActiveMods('new-scroll-from');
    const toMods = getActiveMods('new-scroll-to');
    const global = document.getElementById('new-scroll-global').checked;

    config.scroll_rules.push({ from_mods: fromMods, to_mods: toMods, global });
    document.getElementById('new-scroll-global').checked = false;
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

function modsEqual(a, b) {
    if (a.length !== b.length) return false;
    const sa = [...a].sort();
    const sb = [...b].sort();
    return sa.every((m, i) => m === sb[i]);
}

function validateKeyRules(rules) {
    const warnings = [];
    const batchRules = rules.filter(r => r.keys);

    for (let i = 0; i < batchRules.length; i++) {
        for (let j = i + 1; j < batchRules.length; j++) {
            const a = batchRules[i], b = batchRules[j];
            if (!modsEqual(a.from_mods, b.from_mods)) continue;

            const overlap = a.keys.filter(k => b.keys.includes(k));
            if (overlap.length > 0) {
                warnings.push(
                    `Keys [${overlap.join(', ')}] appear in two [${a.from_mods.join('+')}] rules with different targets — only the first rule will fire`
                );
            }

            if (modsEqual(a.to_mods, b.to_mods)) {
                warnings.push(
                    `Two batch rules [${a.from_mods.join('+')}] → [${a.to_mods.join('+')}] could be merged into one`
                );
            }
        }
    }

    // Check single rules shadowed by batch rules
    const singleRules = rules.filter(r => r.from_key);
    for (const single of singleRules) {
        for (const batch of batchRules) {
            if (modsEqual(single.from_mods, batch.from_mods) && batch.keys.includes(single.from_key)) {
                warnings.push(
                    `Single rule [${single.from_mods.join('+')}]+${single.from_key} is shadowed by an earlier batch rule`
                );
            }
        }
    }

    return warnings;
}

function showWarnings(warnings) {
    const el = document.getElementById('save-warnings');
    if (warnings.length === 0) {
        el.innerHTML = '';
        return;
    }
    el.innerHTML = warnings.map(w => `<span class="warning-line">⚠ ${w}</span>`).join('')
        + '<span class="warning-hint">Click save again to confirm.</span>';
}

async function saveConfig() {
    const warnings = validateKeyRules(config.key_rules);

    if (warnings.length > 0 && !pendingWarnings) {
        showWarnings(warnings);
        pendingWarnings = true;
        return;
    }

    pendingWarnings = false;
    showWarnings([]);

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

// --- Schema selector ---
const SCHEMAS_BASE = 'schemas';

async function loadSchemaManifest() {
    const select = document.getElementById('schema-select');
    if (!select) return;

    try {
        const res = await fetch(`${SCHEMAS_BASE}/manifest.json`);
        if (!res.ok) return;
        const manifest = await res.json();

        manifest.forEach(s => {
            const opt = document.createElement('option');
            opt.value = s.id;
            opt.textContent = s.name;
            select.appendChild(opt);
        });

        // Auto-detect from browser language
        const lang = (navigator.language || '').split('-')[0].toLowerCase();
        const match = manifest.find(s => s.lang.includes(lang));
        if (match) {
            select.value = match.id;
        }
    } catch (e) {
        console.warn('Could not load schema manifest', e);
    }
}

document.getElementById('apply-schema-btn')?.addEventListener('click', async () => {
    const select = document.getElementById('schema-select');
    const schemaId = select?.value;
    if (!schemaId) return;

    try {
        const res = await fetch(`${SCHEMAS_BASE}/${schemaId}.json`);
        if (!res.ok) throw new Error(`Failed to load schema ${schemaId}`);
        const rules = await res.json();

        let added = 0;
        for (const rule of rules) {
            const dup = config.char_rules.some(r =>
                JSON.stringify(r.from_mods) === JSON.stringify(rule.from_mods) &&
                r.from_key === rule.from_key
            );
            if (!dup) {
                config.char_rules.push({
                    from_mods: normalizeMods(rule.from_mods),
                    from_key: String(rule.from_key),
                    to_char: String(rule.to_char),
                    global: !!rule.global,
                });
                added++;
            }
        }

        renderCharRules();
        const status = document.getElementById('schema-status');
        if (status) {
            status.textContent = added > 0 ? `Added ${added} rules` : 'No new rules (all exist)';
            setTimeout(() => { status.textContent = ''; }, 2000);
        }
    } catch (e) {
        console.error('Failed to apply schema', e);
    }
});

loadSchemaManifest();

loadConfig();
