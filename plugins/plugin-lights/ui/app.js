import { createHueWheel, hexToHueSat } from './components/hue-wheel.js';

const PLUGIN_ID = window.location.pathname.split('/').filter(Boolean)[1];
const CONFIG_URL = `/api/plugins/${PLUGIN_ID}/config`;
const PERMISSIONS_URL = `/api/plugins/${PLUGIN_ID}/permissions`;
const REQUEST_PERMISSION_URL = `/api/permissions/serial/request`;
const ACTION_URL = `/api/plugins/${PLUGIN_ID}/actions`;

let config = null;
let permissionStatus = { permissions: {} };
let permissionsLoaded = false;
let ensuringPermissions = false;
let pairCountdown = 0;
let pairTimer = null;
let actionMessage = '';
let permissionMessage = '';
let refreshTimer = null;
let hueWheelInstance = null;
let controlsSection = null;
let controlsDeviceId = null;
let serialPortDraft = null;
let savingSerialPort = false;

function parsePortPaths(value) {
    return value.match(/\/dev\/[^,\s]+/g) || [];
}

function parsePortDescriptions(value) {
    return value.match(/\/dev\/[^,\s]+(?: \[[^\]]+\])?/g) || [];
}

function coordinatorIssue() {
    if (typeof actionMessage !== 'string') return null;

    const message = actionMessage.trim();
    if (!message) return null;

    const respondedPrefix = 'no Zigbee coordinator responded on auto-detected serial ports: ';
    if (message.startsWith(respondedPrefix)) {
        const rest = message.slice(respondedPrefix.length);
        const [candidatesPart, devicesPart = ''] = rest.split('; available serial devices: ');
        const candidatePorts = parsePortPaths(candidatesPart);
        const availablePorts = parsePortDescriptions(devicesPart);
        const checkedCount = candidatePorts.length;
        const checkedLabel = checkedCount === 1 ? 'port' : 'ports';
        return {
            title: 'Coordinator not connected',
            detail: checkedCount
                ? `Auto-detect checked ${checkedCount} likely ${checkedLabel}`
                : 'Auto-detect could not verify a coordinator',
            summary: checkedCount
                ? `Checked ${checkedCount} likely ${checkedLabel}, but none answered a Zigbee probe.`
                : 'Auto-detect could not verify a coordinator.',
            candidatePorts,
            availablePorts,
        };
    }

    const supportedPrefix = 'no supported Zigbee coordinator detected automatically; available serial devices: ';
    if (message.startsWith(supportedPrefix)) {
        return {
            title: 'Coordinator not detected',
            detail: 'Auto-detect did not find a likely Zigbee coordinator',
            summary: 'No serial device identified itself clearly enough to auto-select.',
            candidatePorts: [],
            availablePorts: parsePortDescriptions(message.slice(supportedPrefix.length)),
        };
    }

    if (message === 'no supported Zigbee coordinator detected automatically and no serial devices are available') {
        return {
            title: 'No serial devices available',
            detail: 'No serial devices are visible to the plugin',
            summary: 'Connect a Zigbee coordinator or make sure serial access is available.',
            candidatePorts: [],
            availablePorts: [],
        };
    }

    return null;
}

function defaultPreset(name) {
    return {
        enabled: false,
        name,
        power_on: true,
        brightness: 100,
        color_hex: 'ffffff',
        mirek: 300,
    };
}

function defaultConfig() {
    return {
        backend: {
            kind: 'zigbee-direct',
            serial_port: 'auto',
            channel: 11,
            network_key: 'auto',
        },
        main_target_type: 'device',
        main_target_id: '',
        devices: {},
        live_color_hex: 'ffffff',
        live_brightness: 100,
        live_mirek: 300,
        presets: {
            preset_1: defaultPreset('Preset 1'),
            preset_2: defaultPreset('Preset 2'),
            preset_3: defaultPreset('Preset 3'),
            preset_4: defaultPreset('Preset 4'),
            preset_5: defaultPreset('Preset 5'),
            preset_6: defaultPreset('Preset 6'),
            preset_7: defaultPreset('Preset 7'),
            preset_8: defaultPreset('Preset 8'),
        },
    };
}

function isRecord(value) {
    return !!value && typeof value === 'object' && !Array.isArray(value);
}

function normalizeConfig(raw) {
    const fallback = defaultConfig();
    const source = isRecord(raw) ? raw : {};
    const backend = isRecord(source.backend) ? source.backend : {};
    const presets = isRecord(source.presets) ? source.presets : {};
    return {
        ...fallback,
        ...source,
        backend: {
            ...fallback.backend,
            ...backend,
        },
        devices: isRecord(source.devices) ? source.devices : {},
        presets: {
            ...fallback.presets,
            preset_1: { ...fallback.presets.preset_1, ...(isRecord(presets.preset_1) ? presets.preset_1 : {}) },
            preset_2: { ...fallback.presets.preset_2, ...(isRecord(presets.preset_2) ? presets.preset_2 : {}) },
            preset_3: { ...fallback.presets.preset_3, ...(isRecord(presets.preset_3) ? presets.preset_3 : {}) },
            preset_4: { ...fallback.presets.preset_4, ...(isRecord(presets.preset_4) ? presets.preset_4 : {}) },
            preset_5: { ...fallback.presets.preset_5, ...(isRecord(presets.preset_5) ? presets.preset_5 : {}) },
            preset_6: { ...fallback.presets.preset_6, ...(isRecord(presets.preset_6) ? presets.preset_6 : {}) },
            preset_7: { ...fallback.presets.preset_7, ...(isRecord(presets.preset_7) ? presets.preset_7 : {}) },
            preset_8: { ...fallback.presets.preset_8, ...(isRecord(presets.preset_8) ? presets.preset_8 : {}) },
        },
    };
}

function configNeedsBootstrap(raw) {
    if (!isRecord(raw)) return true;
    return !isRecord(raw.backend);
}

async function loadConfig() {
    try {
        const res = await fetch(CONFIG_URL);
        const raw = res.ok ? await res.json() : null;
        config = normalizeConfig(raw);
        if (serialPortDraft === null) {
            serialPortDraft = config.backend?.serial_port || 'auto';
        }
        render();
        if (!configNeedsBootstrap(raw)) return;
        await saveConfig(config);
    } catch (_) {}
}

async function loadPermissions() {
    try {
        const res = await fetch(PERMISSIONS_URL);
        if (!res.ok) {
            permissionsLoaded = true;
            render();
            return;
        }
        permissionStatus = await res.json();
        permissionsLoaded = true;
        render();
    } catch (_) {
        permissionsLoaded = true;
        render();
    }
}

async function refreshData() {
    await Promise.all([
        loadPermissions(),
        loadConfig(),
    ]);
}

function permissionsBlocked() {
    if (!permissionsLoaded) return false;
    return Object.values(permissionStatus.permissions)
        .some(p => p.state !== 'granted');
}

function worstPermissionState() {
    const unmet = Object.values(permissionStatus.permissions)
        .filter(p => p.state !== 'granted');
    if (unmet.length === 0) return 'granted';
    if (unmet.some(p => p.state === 'denied')) return 'denied';
    if (unmet.some(p => p.state === 'requires_logout')) return 'requires_logout';
    return 'fixable';
}

async function requestPermissions() {
    if (ensuringPermissions) return;
    ensuringPermissions = true;
    permissionMessage = '';
    render();

    try {
        const res = await fetch(REQUEST_PERMISSION_URL, { method: 'POST' });
        if (!res.ok) {
            permissionMessage = res.status === 400
                ? 'Permission already granted.'
                : 'Could not request permissions.';
            return;
        }
        const result = await res.json();
        if (result.state === 'requires_logout') {
            permissionMessage = result.hint || 'Log out and back in to activate serial access';
        } else if (result.state === 'denied') {
            permissionMessage = result.hint || 'Could not configure serial access';
        } else {
            permissionMessage = '';
        }
        await loadPermissions();
    } catch (_) {
        permissionMessage = 'Could not request permissions.';
    } finally {
        ensuringPermissions = false;
        render();
    }
}

async function saveConfig(updated) {
    try {
        const res = await fetch(CONFIG_URL, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(updated),
        });
        if (!res.ok) return false;
        config = updated;
        serialPortDraft = updated.backend?.serial_port || 'auto';
        render();
        return true;
    } catch (_) {}

    return false;
}

async function silentSaveConfig(updated) {
    try {
        const res = await fetch(CONFIG_URL, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(updated),
        });
        if (res.ok) config = updated;
    } catch (_) {}
}

async function sendAction(action, btn) {
    try {
        const res = await fetch(`${ACTION_URL}/${action}`, { method: 'POST' });
        const body = await res.json().catch(() => null);
        const success = res.ok && body?.success !== false;
        if (!success) {
            actionMessage = body?.message || 'Action failed.';
            render();
            return false;
        }

        actionMessage = '';
        if (btn) flashButton(btn);
        render();
        return true;
    } catch (_) {
        actionMessage = 'Action failed.';
        render();
        return false;
    }
}

function flashButton(el) {
    el.classList.remove('flash');
    void el.offsetWidth;
    el.classList.add('flash');
}

async function startPairing() {
    const started = await sendAction('pair');
    if (!started) return;

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

function currentSerialPortDraft() {
    return serialPortDraft ?? config?.backend?.serial_port ?? 'auto';
}

async function saveSerialPort() {
    if (savingSerialPort || !config) return;

    savingSerialPort = true;
    actionMessage = '';
    render();

    const serialPort = currentSerialPortDraft().trim() || 'auto';
    const success = await saveConfig({
        ...config,
        backend: {
            ...(config.backend || {}),
            serial_port: serialPort,
        },
    });

    savingSerialPort = false;
    if (!success) {
        actionMessage = 'Could not save connection settings.';
    }
    render();
}

function buildConnectionSection() {
    const section = document.createElement('div');
    section.className = 'section';

    const title = document.createElement('div');
    title.className = 'section-title';
    title.textContent = 'Connection';
    section.appendChild(title);

    const label = document.createElement('label');
    label.className = 'field-label';
    label.htmlFor = 'serial-port-input';
    label.textContent = 'Serial Port';
    section.appendChild(label);

    const row = document.createElement('div');
    row.className = 'field-row';

    const input = document.createElement('input');
    input.className = 'text-input';
    input.id = 'serial-port-input';
    input.type = 'text';
    input.spellcheck = false;
    input.placeholder = 'auto or /dev/cu.usbserial...';
    input.value = currentSerialPortDraft();
    input.addEventListener('input', event => {
        serialPortDraft = event.target.value;
    });
    input.addEventListener('keydown', event => {
        if (event.key !== 'Enter') return;
        event.preventDefault();
        saveSerialPort();
    });
    row.appendChild(input);

    const saveButton = document.createElement('button');
    saveButton.className = 'btn btn-sm btn-primary';
    saveButton.textContent = savingSerialPort ? 'Saving...' : 'Save';
    saveButton.disabled = savingSerialPort;
    saveButton.addEventListener('click', saveSerialPort);
    row.appendChild(saveButton);

    const autoButton = document.createElement('button');
    autoButton.className = 'btn btn-sm btn-ghost';
    autoButton.textContent = 'Auto';
    autoButton.disabled = savingSerialPort;
    autoButton.addEventListener('click', () => {
        serialPortDraft = 'auto';
        saveSerialPort();
    });
    row.appendChild(autoButton);

    section.appendChild(row);

    const hint = document.createElement('div');
    hint.className = 'field-hint';
    hint.textContent = 'Use auto for clearly identified Zigbee dongles. Set an explicit /dev/... path when the coordinator appears as a generic USB serial device.';
    section.appendChild(hint);

    return section;
}

function backendSummary() {
    const backend = config.backend || {};
    return `${backend.kind || 'unknown'} \u00B7 port: ${backend.serial_port || 'auto'} \u00B7 ch ${backend.channel || '?'}`;
}

function blockedStatusState() {
    const requiresRestart = worstPermissionState() === 'requires_logout';
    return {
        connected: false,
        title: requiresRestart ? 'Restart required' : 'Permission required',
        detail: requiresRestart
            ? 'Log out and back in to activate serial access'
            : 'Grant serial access to connect the Zigbee dongle',
        note: '',
        issue: null,
        action: '',
    };
}

function issueStatusState(issue) {
    const backend = config.backend || {};
    return {
        connected: false,
        title: issue.title,
        detail: `${issue.detail} · port: ${backend.serial_port || 'auto'} · ch ${backend.channel || '?'}`,
        note: permissionMessage,
        issue,
        action: '',
    };
}

function defaultStatusState() {
    const hasMain = !!config.main_target_id;
    return {
        connected: hasMain,
        title: hasMain ? 'Main target configured' : 'No main target set',
        detail: backendSummary(),
        note: permissionMessage,
        issue: null,
        action: actionMessage,
    };
}

function statusState() {
    if (permissionsBlocked()) {
        return blockedStatusState();
    }

    const issue = coordinatorIssue();
    if (issue) {
        return issueStatusState(issue);
    }

    return defaultStatusState();
}

function buildStatusHeader(state) {
    const dot = document.createElement('span');
    dot.className = `status-dot ${state.connected ? 'connected' : 'warning'}`;

    const text = document.createElement('span');
    text.className = 'status-text';
    text.textContent = state.title;

    const detail = document.createElement('span');
    detail.className = 'status-detail';
    detail.textContent = state.detail;

    const copy = document.createElement('div');
    copy.className = 'status-copy';
    copy.append(text, detail);

    const bar = document.createElement('div');
    bar.className = 'status-bar';
    bar.append(dot, copy);
    return bar;
}

function buildStatusNote(message, error = false) {
    if (!message) return null;

    const note = document.createElement('div');
    note.className = error ? 'status-note status-note-error' : 'status-note';
    note.textContent = message;
    return note;
}

function buildPortGroup(labelText, ports, muted = false) {
    if (!ports.length) return null;

    const group = document.createElement('div');
    group.className = 'status-diagnostic-group';

    const label = document.createElement('div');
    label.className = 'status-diagnostic-label';
    label.textContent = labelText;
    group.appendChild(label);

    const list = document.createElement('div');
    list.className = 'status-port-list';
    ports.forEach(port => {
        const entry = document.createElement('div');
        entry.className = muted ? 'status-port-entry status-port-entry-muted' : 'status-port-entry';
        entry.textContent = port;
        list.appendChild(entry);
    });
    group.appendChild(list);
    return group;
}

function buildIssueDiagnostics(issue) {
    if (!issue) return null;
    if (!issue.candidatePorts.length && !issue.availablePorts.length) return null;

    const diagnostics = document.createElement('details');
    diagnostics.className = 'status-diagnostics';

    const summary = document.createElement('summary');
    summary.textContent = 'Serial diagnostics';
    diagnostics.appendChild(summary);

    const body = document.createElement('div');
    body.className = 'status-diagnostics-body';
    const checkedPorts = buildPortGroup('Checked ports', issue.candidatePorts);
    if (checkedPorts) {
        body.appendChild(checkedPorts);
    }
    const detectedDevices = buildPortGroup('Detected serial devices', issue.availablePorts, true);
    if (detectedDevices) {
        body.appendChild(detectedDevices);
    }
    diagnostics.appendChild(body);
    return diagnostics;
}

function appendStatusContent(section, state) {
    const note = buildStatusNote(state.note);
    if (note) {
        section.appendChild(note);
    }

    if (state.issue) {
        const issueNote = buildStatusNote(state.issue.summary, true);
        if (issueNote) {
            section.appendChild(issueNote);
        }
        const diagnostics = buildIssueDiagnostics(state.issue);
        if (diagnostics) {
            section.appendChild(diagnostics);
        }
        return;
    }

    const action = buildStatusNote(state.action, true);
    if (action) {
        section.appendChild(action);
    }
}

function buildStatusBar() {
    const state = statusState();

    const section = document.createElement('div');
    section.className = 'section';
    section.appendChild(buildStatusHeader(state));
    appendStatusContent(section, state);
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
    const active = !!config.main_target_id;

    if (controlsSection && controlsDeviceId === config.main_target_id) {
        return controlsSection;
    }

    if (hueWheelInstance) {
        hueWheelInstance.destroy();
        hueWheelInstance = null;
    }

    controlsDeviceId = config.main_target_id;

    const section = document.createElement('div');
    section.className = 'section';

    const title = document.createElement('div');
    title.className = 'section-title';
    title.textContent = 'Controls';
    section.appendChild(title);

    if (!active) {
        const hint = document.createElement('div');
        hint.className = 'controls-hint';
        hint.textContent = 'Set a main device to enable controls';
        section.appendChild(hint);
        controlsSection = section;
        return section;
    }

    const grid = document.createElement('div');
    grid.className = 'controls-grid';

    const toggleBtn = document.createElement('button');
    toggleBtn.className = 'btn btn-accent toggle-btn';
    toggleBtn.textContent = 'Toggle';
    toggleBtn.addEventListener('click', function () { sendAction('toggle-main', this); });
    grid.appendChild(toggleBtn);

    const wheelContainer = document.createElement('div');
    grid.appendChild(wheelContainer);

    const { hue, saturation } = hexToHueSat(config.live_color_hex || 'ffffff');

    hueWheelInstance = createHueWheel(wheelContainer, {
        onRelease({ hex, brightness: level }) {
            config.live_color_hex = hex;
            config.live_brightness = level;
            silentSaveConfig(config);
        },
        initialState: {
            hue,
            saturation,
            brightness: config.live_brightness ?? 100,
        },
    });

    section.appendChild(grid);
    controlsSection = section;
    return section;
}


function permissionCopy() {
    const unmet = Object.entries(permissionStatus.permissions)
        .filter(([_, p]) => p.state !== 'granted')
        .map(([name]) => name);
    const required = unmet.length ? unmet.join(', ') : 'hardware access';
    return `Lights needs ${required} before it can talk to the Zigbee dongle. The OS prompt only appears after you click below.`;
}

function buildPermissionBackdrop() {
    if (!permissionsBlocked()) return null;

    const backdrop = document.createElement('div');
    backdrop.className = 'permission-backdrop';

    const card = document.createElement('div');
    card.className = 'permission-card';

    const worstState = worstPermissionState();

    const eyebrow = document.createElement('div');
    eyebrow.className = 'permission-eyebrow';
    eyebrow.textContent = worstState === 'requires_logout' ? 'Restart required' : 'Permission required';

    const title = document.createElement('h1');
    title.className = 'permission-title';
    title.textContent = worstState === 'requires_logout' ? 'Log Out Required' : 'Give Permissions';

    const copy = document.createElement('p');
    copy.className = 'permission-copy';
    copy.textContent = worstState === 'requires_logout'
        ? 'Serial access has been configured but requires a new login session to take effect.'
        : permissionCopy();

    const meta = document.createElement('div');
    meta.className = 'permission-meta';
    const unmet = Object.entries(permissionStatus.permissions)
        .filter(([_, p]) => p.state !== 'granted');
    meta.textContent = unmet.length
        ? `Missing: ${unmet.map(([name]) => name).join(', ')}`
        : 'Missing access';

    card.append(eyebrow, title, copy, meta);

    if (worstState !== 'requires_logout') {
        const actionRow = document.createElement('div');
        actionRow.className = 'permission-actions';

        const button = document.createElement('button');
        button.className = 'btn btn-accent permission-btn';
        button.textContent = ensuringPermissions
            ? 'Requesting...'
            : worstState === 'denied' ? 'Retry' : 'Give Permissions';
        button.disabled = ensuringPermissions;
        button.addEventListener('click', requestPermissions);
        actionRow.appendChild(button);
        card.appendChild(actionRow);
    }

    if (permissionMessage) {
        const note = document.createElement('div');
        note.className = 'permission-note';
        note.textContent = permissionMessage;
        card.appendChild(note);
    }

    backdrop.appendChild(card);
    return backdrop;
}

function render() {
    const app = document.getElementById('app');
    app.replaceChildren();

    if (!config || !permissionsLoaded) {
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
    container.className = `container${permissionsBlocked() ? ' page-blocked' : ''}`;
    container.append(
        buildStatusBar(),
        buildConnectionSection(),
        buildDevices(),
        buildControls(),
    );
    app.appendChild(container);

    const backdrop = buildPermissionBackdrop();
    if (!backdrop) return;

    app.appendChild(backdrop);
}

refreshData();
refreshTimer = setInterval(refreshData, 5000);
