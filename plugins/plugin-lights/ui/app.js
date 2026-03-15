const PLUGIN_ID = window.location.pathname.split('/').filter(Boolean)[1];
const CONFIG_URL = `/api/plugins/${PLUGIN_ID}/config`;
const ACTION_URL = `/api/plugins/${PLUGIN_ID}/actions`;

let config = null;
let editingPreset = null;
let pairCountdown = 0;
let pairTimer = null;
let refreshTimer = null;

async function loadConfig() {
    try {
        const res = await fetch(CONFIG_URL);
        if (!res.ok) return;
        config = await res.json();
        render();
    } catch (_) {}
}

async function saveConfig(updated) {
    try {
        const res = await fetch(CONFIG_URL, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(updated),
        });
        if (!res.ok) return;
        config = updated;
        render();
    } catch (_) {}
}

async function sendAction(action, btn) {
    try {
        await fetch(`${ACTION_URL}/${action}`, { method: 'POST' });
        if (btn) flashButton(btn);
    } catch (_) {}
}

function flashButton(el) {
    el.classList.remove('flash');
    void el.offsetWidth;
    el.classList.add('flash');
}

function startPairing() {
    sendAction('pair');
    pairCountdown = 60;
    if (pairTimer) clearInterval(pairTimer);
    pairTimer = setInterval(() => {
        pairCountdown--;
        if (pairCountdown <= 0) {
            clearInterval(pairTimer);
            pairTimer = null;
        }
        renderPairStatus();
    }, 1000);
    renderPairStatus();
}

function renderPairStatus() {
    const el = document.getElementById('pair-status');
    if (!el) return;
    el.textContent = pairCountdown > 0 ? `Pairing... (${pairCountdown}s)` : '';
}

function setMainDevice(deviceId) {
    saveConfig({ ...config, main_target_id: deviceId });
}

function triggerPreset(n, e) {
    const preset = config.presets[`preset_${n}`];
    if (!preset?.enabled) return;
    sendAction(`preset-${n}`, e.currentTarget);
}

function togglePresetEditor(n, e) {
    e.stopPropagation();
    editingPreset = editingPreset === n ? null : n;
    render();
}

function savePreset(n) {
    const key = `preset_${n}`;
    const form = document.getElementById(`preset-form-${n}`);
    if (!form) return;

    const updated = {
        ...config,
        presets: {
            ...config.presets,
            [key]: {
                enabled: form.querySelector('[data-field="enabled"]').checked,
                name: form.querySelector('[data-field="name"]').value,
                power_on: config.presets[key].power_on,
                brightness: parseInt(form.querySelector('[data-field="brightness"]').value, 10) || 100,
                color_hex: form.querySelector('[data-field="color_hex"]').value.replace('#', ''),
                mirek: parseInt(form.querySelector('[data-field="mirek"]').value, 10) || 300,
            },
        },
    };
    editingPreset = null;
    saveConfig(updated);
}

function escHtml(s) {
    const el = document.createElement('span');
    el.textContent = s;
    return el.innerHTML;
}

function escAttr(s) {
    return String(s).replace(/&/g, '&amp;').replace(/"/g, '&quot;')
        .replace(/'/g, '&#39;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function buildStatusBar() {
    const backend = config.backend || {};
    const hasMain = !!config.main_target_id;
    const dot = document.createElement('span');
    dot.className = `status-dot ${hasMain ? 'connected' : 'warning'}`;

    const text = document.createElement('span');
    text.className = 'status-text';
    text.textContent = hasMain ? 'Main target configured' : 'No main target set';

    const detail = document.createElement('span');
    detail.className = 'status-detail';
    detail.textContent = `${backend.kind || 'unknown'} \u00B7 port: ${backend.serial_port || 'auto'} \u00B7 ch ${backend.channel || '?'}`;

    const bar = document.createElement('div');
    bar.className = 'status-bar';
    bar.append(dot, text, detail);

    const section = document.createElement('div');
    section.className = 'section';
    section.appendChild(bar);
    return section;
}

function buildDeviceRow(id, dev, isMain) {
    const row = document.createElement('div');
    row.className = `device-row${isMain ? ' active' : ''}`;

    const info = document.createElement('div');
    const nameRow = document.createElement('div');
    nameRow.className = 'device-name';
    const statusDot = document.createElement('span');
    statusDot.className = `dot ${dev.online ? 'dot-green' : 'dot-red'}`;
    const nameText = document.createTextNode(dev.name || id);
    nameRow.append(statusDot, nameText);
    const addr = document.createElement('div');
    addr.className = 'device-addr';
    addr.textContent = `${id} ${dev.online ? '' : '(offline)'}`;
    info.append(nameRow, addr);

    const actions = document.createElement('div');
    actions.className = 'device-actions';
    if (isMain) {
        const badge = document.createElement('span');
        badge.className = 'main-badge';
        badge.textContent = 'Main';
        actions.appendChild(badge);
    } else {
        const btn = document.createElement('button');
        btn.className = 'btn btn-sm btn-primary';
        btn.textContent = 'Set as Main';
        btn.addEventListener('click', () => setMainDevice(id));
        actions.appendChild(btn);
    }

    row.append(info, actions);
    return row;
}

function buildDevices() {
    const section = document.createElement('div');
    section.className = 'section';

    const title = document.createElement('div');
    title.className = 'section-title';
    title.textContent = 'Devices';
    section.appendChild(title);

    const list = document.createElement('div');
    list.className = 'device-list';

    const devices = config.devices || {};
    const entries = Object.entries(devices);
    const mainId = config.main_target_id || '';

    if (entries.length === 0) {
        const empty = document.createElement('div');
        empty.className = 'no-devices';
        empty.textContent = 'No devices found. Pair a device to get started.';
        list.appendChild(empty);
    } else {
        entries.forEach(([id, dev]) => {
            list.appendChild(buildDeviceRow(id, dev, id === mainId));
        });
    }

    section.appendChild(list);

    const pairRow = document.createElement('div');
    pairRow.className = 'pair-row';

    const pairBtn = document.createElement('button');
    pairBtn.className = 'btn btn-sm btn-ghost';
    pairBtn.textContent = 'Pair New Device';
    pairBtn.disabled = pairCountdown > 0;
    pairBtn.addEventListener('click', startPairing);

    const pairStatus = document.createElement('span');
    pairStatus.className = 'pair-status';
    pairStatus.id = 'pair-status';
    pairStatus.textContent = pairCountdown > 0 ? `Pairing... (${pairCountdown}s)` : '';

    pairRow.append(pairBtn, pairStatus);
    section.appendChild(pairRow);
    return section;
}

function buildControls() {
    const section = document.createElement('div');
    section.className = 'section';

    const title = document.createElement('div');
    title.className = 'section-title';
    title.textContent = 'Controls';
    section.appendChild(title);

    const active = !!config.main_target_id;

    if (!active) {
        const hint = document.createElement('div');
        hint.className = 'controls-hint';
        hint.textContent = 'Set a main device to enable controls';
        section.appendChild(hint);
    }

    const grid = document.createElement('div');
    grid.className = `controls-grid${active ? '' : ' controls-disabled'}`;

    const toggleBtn = document.createElement('button');
    toggleBtn.className = 'btn btn-accent toggle-btn';
    toggleBtn.textContent = 'Toggle';
    toggleBtn.disabled = !active;
    toggleBtn.addEventListener('click', function () { sendAction('toggle-main', this); });
    grid.appendChild(toggleBtn);

    const brightnessRow = buildControlRow('Brightness', [
        { label: '\u2212 Dimmer', action: 'dimmer-main' },
        { label: '+ Brighter', action: 'brighter-main' },
    ], active);
    grid.appendChild(brightnessRow);

    const tempRow = buildControlRow('Color Temp', [
        { label: 'Warmer', action: 'warmer-main' },
        { label: 'Cooler', action: 'cooler-main' },
    ], active);
    grid.appendChild(tempRow);

    section.appendChild(grid);
    return section;
}

function buildControlRow(labelText, buttons, active) {
    const row = document.createElement('div');
    row.className = 'control-row';

    const label = document.createElement('span');
    label.className = 'control-label';
    label.textContent = labelText;
    row.appendChild(label);

    const group = document.createElement('div');
    group.className = 'btn-group';

    buttons.forEach(({ label: btnLabel, action }) => {
        const btn = document.createElement('button');
        btn.className = 'btn btn-primary';
        btn.textContent = btnLabel;
        btn.disabled = !active;
        btn.addEventListener('click', function () { sendAction(action, this); });
        group.appendChild(btn);
    });

    row.appendChild(group);
    return row;
}

function buildPresetCard(n) {
    const key = `preset_${n}`;
    const preset = config.presets?.[key];
    if (!preset) return null;

    const isEditing = editingPreset === n;

    const card = document.createElement('div');
    card.className = `preset-card${preset.enabled ? '' : ' disabled'}`;
    card.addEventListener('click', (e) => triggerPreset(n, e));

    const header = document.createElement('div');
    header.className = 'preset-header';

    const nameEl = document.createElement('span');
    nameEl.className = 'preset-name';
    nameEl.textContent = preset.name;

    const editBtn = document.createElement('button');
    editBtn.className = 'preset-edit-btn';
    editBtn.innerHTML = '&#x21B5;';
    editBtn.title = 'Edit preset';
    editBtn.addEventListener('click', (e) => togglePresetEditor(n, e));

    header.append(nameEl, editBtn);
    card.appendChild(header);

    const info = document.createElement('div');
    info.className = 'preset-info';

    const swatch = document.createElement('span');
    swatch.className = 'preset-swatch';
    swatch.style.background = `#${preset.color_hex || 'ffffff'}`;

    const meta = document.createElement('span');
    meta.textContent = `${preset.brightness}% \u00B7 ${preset.mirek}K`;

    info.append(swatch, meta);
    card.appendChild(info);

    if (isEditing) {
        card.appendChild(buildPresetEditor(n, preset));
    }

    return card;
}

function buildPresetEditor(n, preset) {
    const editor = document.createElement('div');
    editor.className = 'preset-editor';
    editor.id = `preset-form-${n}`;
    editor.addEventListener('click', (e) => e.stopPropagation());

    const fields = [
        { label: 'Name', field: 'name', type: 'text', value: preset.name },
        { label: 'Brightness', field: 'brightness', type: 'number', value: preset.brightness, min: 0, max: 100 },
        { label: 'Color', field: 'color_hex', type: 'text', value: preset.color_hex, placeholder: 'ffffff' },
        { label: 'Mirek', field: 'mirek', type: 'number', value: preset.mirek, min: 153, max: 500 },
    ];

    fields.forEach(({ label, field, type, value, min, max, placeholder }) => {
        const row = document.createElement('div');
        row.className = 'field-row';

        const lbl = document.createElement('span');
        lbl.className = 'field-label';
        lbl.textContent = label;

        const input = document.createElement('input');
        input.type = type;
        input.dataset.field = field;
        input.value = value;
        if (min !== undefined) input.min = min;
        if (max !== undefined) input.max = max;
        if (placeholder) input.placeholder = placeholder;

        row.append(lbl, input);
        editor.appendChild(row);
    });

    const toggleRow = document.createElement('div');
    toggleRow.className = 'toggle-row';

    const checkbox = document.createElement('input');
    checkbox.type = 'checkbox';
    checkbox.id = `preset-enabled-${n}`;
    checkbox.dataset.field = 'enabled';
    checkbox.checked = preset.enabled;

    const toggleLabel = document.createElement('label');
    toggleLabel.htmlFor = `preset-enabled-${n}`;
    toggleLabel.textContent = 'Enabled';

    toggleRow.append(checkbox, toggleLabel);
    editor.appendChild(toggleRow);

    const actions = document.createElement('div');
    actions.className = 'editor-actions';

    const saveBtn = document.createElement('button');
    saveBtn.className = 'btn btn-sm btn-primary';
    saveBtn.textContent = 'Save';
    saveBtn.addEventListener('click', () => savePreset(n));

    const cancelBtn = document.createElement('button');
    cancelBtn.className = 'btn btn-sm btn-ghost';
    cancelBtn.textContent = 'Cancel';
    cancelBtn.addEventListener('click', (e) => togglePresetEditor(n, e));

    actions.append(saveBtn, cancelBtn);
    editor.appendChild(actions);
    return editor;
}

function buildPresets() {
    const section = document.createElement('div');
    section.className = 'section';

    const title = document.createElement('div');
    title.className = 'section-title';
    title.textContent = 'Presets';
    section.appendChild(title);

    const grid = document.createElement('div');
    grid.className = 'preset-grid';

    for (let i = 1; i <= 8; i++) {
        const card = buildPresetCard(i);
        if (card) grid.appendChild(card);
    }

    section.appendChild(grid);
    return section;
}

function render() {
    const app = document.getElementById('app');
    app.replaceChildren();

    if (!config) {
        const loading = document.createElement('div');
        loading.className = 'container';
        const sec = document.createElement('div');
        sec.className = 'section';
        sec.textContent = 'Loading...';
        loading.appendChild(sec);
        app.appendChild(loading);
        return;
    }

    const container = document.createElement('div');
    container.className = 'container';
    container.append(
        buildStatusBar(),
        buildDevices(),
        buildControls(),
        buildPresets(),
    );
    app.appendChild(container);
}

loadConfig();
refreshTimer = setInterval(loadConfig, 5000);
