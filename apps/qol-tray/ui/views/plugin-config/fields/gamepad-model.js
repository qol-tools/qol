import { hapticCapability } from './gamepad-haptics.js';

export const STANDARD_BUTTON_NAMES = Object.freeze([
    'A',
    'B',
    'X',
    'Y',
    'LB',
    'RB',
    'LT',
    'RT',
    'View',
    'Menu',
    'L3',
    'R3',
    'D-pad Up',
    'D-pad Down',
    'D-pad Left',
    'D-pad Right',
    'Home',
]);

export const STANDARD_AXIS_NAMES = Object.freeze([
    'Left X',
    'Left Y',
    'Right X',
    'Right Y',
]);

export const INPUT_DEADZONE = 0.08;
export const SIGNAL_HISTORY_LIMIT = 30;
export const SIGNAL_SAMPLE_INTERVAL_MS = 2000;

const SIGNAL_FLOOR_DBM = -95;
const SIGNAL_CEILING_DBM = -35;
const SIGNAL_MINIMUM_HEIGHT = 12;
const BREDR_MARGIN_FLOOR_DB = -20;

const GULIKIT_FIREFOX_BUTTON_MAP = Object.freeze([
    0,
    1,
    18,
    3,
    2,
    19,
    6,
    7,
    4,
    5,
    null,
    null,
    12,
    13,
    14,
    15,
    17,
]);

const EMPTY_BUTTON = Object.freeze({ pressed: false, touched: false, value: 0 });

export function gamepadSnapshot(gamepad) {
    if (!gamepad) return null;
    const standard = gamepad.mapping === 'standard';
    const index = finiteNumber(gamepad.index);
    const rawButtons = Array.from(gamepad.buttons || []);
    const mappingProfile = gulikitFirefoxProfile(gamepad, rawButtons);
    const haptics = hapticCapability(gamepad);
    const buttonSource = mappingProfile
        ? [
            ...GULIKIT_FIREFOX_BUTTON_MAP.map(rawIndex => rawIndex === null
                ? EMPTY_BUTTON
                : rawButtons[rawIndex] || EMPTY_BUTTON),
            ...rawButtons.slice(20),
        ]
        : rawButtons;
    return {
        id: gamepad.id || `Gamepad ${index + 1}`,
        index,
        connected: gamepad.connected !== false,
        mapping: standard ? 'standard' : 'raw',
        mappingProfile,
        nativeInput: null,
        connection: null,
        timestamp: finiteNumber(gamepad.timestamp),
        haptics: Boolean(haptics.mode),
        hapticMode: haptics.mode,
        hapticEffects: haptics.effects,
        buttons: buttonSource.map((button, index) => ({
            index,
            name: standard ? standardButtonName(index) : `B${index}`,
            value: clamp(finiteNumber(button?.value), 0, 1),
            pressed: Boolean(button?.pressed),
            touched: Boolean(button?.touched),
        })),
        axes: Array.from(gamepad.axes || [], (value, index) => ({
            index,
            name: standard ? standardAxisName(index) : `A${index}`,
            value: clamp(finiteNumber(value), -1, 1),
        })),
    };
}

export function mergeNativeInputs(snapshots, payload) {
    if (!Array.isArray(snapshots) || !Array.isArray(payload?.items)) return snapshots;
    const unusedItems = new Set(payload.items.map((_, index) => index));
    return snapshots.map(snapshot => {
        if (snapshot?.mapping !== 'standard') return snapshot;
        const matchIndex = payload.items.findIndex((item, index) =>
            unusedItems.has(index) && nativeItemMatches(snapshot, item));
        if (matchIndex < 0) return snapshot;
        unusedItems.delete(matchIndex);
        return mergeNativeButtons(snapshot, payload.items[matchIndex], payload.source);
    });
}

export function chooseGamepad(gamepads, preference = 'auto') {
    const available = gamepads.filter(gamepad => gamepad?.connected !== false);
    if (available.length === 0) return null;
    if (preference !== 'auto') {
        const preferred = available.find(gamepad => String(gamepad.index) === String(preference));
        if (preferred) return preferred;
    }
    return available.reduce((latest, gamepad) => {
        if (!latest) return gamepad;
        if (gamepad.timestamp > latest.timestamp) return gamepad;
        return latest;
    }, null);
}

export function monitorSignature(gamepads, selected) {
    const inventory = gamepads.map(gamepad => `${gamepad.index}:${gamepad.id}:${gamepad.connected}`).join('|');
    if (!selected) return inventory;
    const buttons = selected.buttons
        .map(button => `${button.pressed ? 1 : 0}:${button.value.toFixed(3)}`)
        .join(',');
    const axes = selected.axes.map(axis => axis.value.toFixed(3)).join(',');
    const connection = selected.connection ? JSON.stringify(selected.connection) : '';
    return `${inventory}/${selected.index}/${selected.timestamp}/${selected.mappingProfile || ''}/${selected.nativeInput || ''}/${selected.hapticMode || ''}/${connection}/${buttons}/${axes}`;
}

export function connectionPresentation(connection) {
    const transport = String(connection?.transport || '').toLowerCase();
    if (!transport) return null;
    if (transport === 'usb') {
        return {
            transport: 'USB',
            detail: 'Wired',
            level: null,
            tone: 'wired',
            signalValue: null,
            signalKind: null,
            valueLabel: null,
            label: 'USB wired connection',
        };
    }
    if (transport !== 'bluetooth') {
        return {
            transport: 'Controller',
            detail: 'Connected',
            level: null,
            tone: 'neutral',
            signalValue: null,
            signalKind: null,
            valueLabel: null,
            label: 'Controller connected',
        };
    }
    const signal = connection.signal;
    if (!signal) {
        return {
            transport: 'Bluetooth',
            detail: 'Signal unavailable',
            level: 0,
            tone: 'neutral',
            signalValue: null,
            signalKind: null,
            valueLabel: null,
            label: 'Bluetooth signal unavailable',
        };
    }
    if (signal.kind === 'bredr_link_margin_db') return bredrConnectionPresentation(signal);
    return absoluteConnectionPresentation(signal);
}

function absoluteConnectionPresentation(signal) {
    const value = Math.round(signal.value);
    const [detail, level, tone] = value >= -55
        ? ['Strong RSSI', 4, 'excellent']
        : value >= -67
            ? ['Usable RSSI', 3, 'good']
            : value >= -78
                ? ['Low RSSI', 2, 'fair']
                : ['Very low RSSI', 1, 'weak'];
    return {
        transport: 'Bluetooth',
        detail,
        level,
        tone,
        signalValue: value,
        signalKind: signal.kind,
        valueLabel: `${value} dBm`,
        label: `Bluetooth reported RSSI ${detail.toLowerCase()}, ${value} dBm. RSSI does not measure packet delivery health.`,
    };
}

function bredrConnectionPresentation(signal) {
    const value = Math.round(signal.value);
    const [detail, level, tone] = value > 0
        ? ['Above target range', 4, 'excellent']
        : value === 0
            ? ['In target range', 4, 'excellent']
            : value >= -5
                ? ['Below target range', 2, 'fair']
                : ['Well below target range', 1, 'weak'];
    return {
        transport: 'Bluetooth',
        detail,
        level,
        tone,
        signalValue: value,
        signalKind: signal.kind,
        valueLabel: relativeValueLabel(value),
        label: `Bluetooth BR/EDR link margin ${detail.toLowerCase()}, ${relativeValueLabel(value)}. This is relative dB, not dBm.`,
    };
}

export function signalHistorySample(connection, bluetoothKnown = false) {
    const signal = connectionPresentation(connection);
    if (signal?.transport === 'Bluetooth') {
        return {
            value: signal.signalValue,
            kind: signal.signalKind,
            strength: signalStrength(signal.signalKind, signal.signalValue),
            tone: signal.tone,
        };
    }
    if (!connection && bluetoothKnown) {
        return { value: null, kind: null, strength: null, tone: 'neutral' };
    }
    return null;
}

export function appendSignalHistory(history, sample, limit = SIGNAL_HISTORY_LIMIT) {
    const current = Array.isArray(history) ? history : [];
    if (!sample) return current;
    const maximum = Number.isInteger(limit) && limit > 0 ? limit : SIGNAL_HISTORY_LIMIT;
    return [...current, sample].slice(-maximum);
}

export function signalHistorySummary(history) {
    if (!Array.isArray(history) || history.length === 0) return null;
    const kind = history.find(sample => sample?.kind)?.kind || null;
    const readings = history
        .filter(sample => (!kind || sample?.kind === kind)
            && sample?.value !== null
            && Number.isFinite(Number(sample?.value)))
        .map(sample => Number(sample.value));
    const unavailableCount = history.length - readings.length;
    const minimum = readings.length > 0 ? Math.min(...readings) : null;
    const maximum = readings.length > 0 ? Math.max(...readings) : null;
    const rangeLabel = signalRangeLabel(kind, minimum, maximum);
    const gapLabel = unavailableCount === 0 ? 'No gaps' : `${unavailableCount} unavailable`;
    const unavailableLabel = unavailableCount === 0
        ? 'no measurements unavailable'
        : unavailableCount === 1
            ? '1 measurement unavailable'
            : `${unavailableCount} measurements unavailable`;
    return {
        count: history.length,
        kind,
        title: kind === 'bredr_link_margin_db' ? 'BR/EDR margin · 60 s' : 'RSSI history · 60 s',
        unavailableCount,
        minimum,
        maximum,
        rangeLabel,
        gapLabel,
        label: `Bluetooth link history: ${history.length} samples, ${rangeLabel}, ${unavailableLabel}`,
    };
}

export function activeInputs(snapshot, deadzone = INPUT_DEADZONE) {
    if (!snapshot) return [];
    const buttons = snapshot.buttons
        .filter(button => button.pressed || button.value > deadzone)
        .map(button => button.value > 0 && button.value < 1
            ? `${button.name} ${formatValue(button.value)}`
            : button.name);
    const axes = snapshot.axes
        .filter(axis => Math.abs(axis.value) > deadzone)
        .map(axis => `${axis.name} ${formatSigned(axis.value)}`);
    return [...buttons, ...axes];
}

export function buttonAt(snapshot, index) {
    return snapshot?.buttons?.[index] || { value: 0, pressed: false, touched: false };
}

export function axisAt(snapshot, index) {
    return snapshot?.axes?.[index]?.value || 0;
}

export function formatSigned(value) {
    const number = finiteNumber(value);
    return `${number >= 0 ? '+' : ''}${number.toFixed(2)}`;
}

export function formatValue(value) {
    return finiteNumber(value).toFixed(2);
}

function standardButtonName(index) {
    return STANDARD_BUTTON_NAMES[index] || `B${index}`;
}

function standardAxisName(index) {
    return STANDARD_AXIS_NAMES[index] || `A${index}`;
}

function gulikitFirefoxProfile(gamepad, buttons) {
    if (gamepad.mapping !== 'standard' || buttons.length < 20) return null;
    const id = String(gamepad.id || '').toLowerCase();
    if (!id.includes('gulikit controller xw')) return null;
    if (!id.includes('045e') || !id.includes('02e0')) return null;
    return 'GuliKit Firefox correction';
}

function nativeItemMatches(snapshot, item) {
    const name = normalizeIdentity(item?.name);
    if (!name) return false;
    return normalizeIdentity(snapshot.id).includes(name);
}

function normalizeIdentity(value) {
    return String(value || '')
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, ' ')
        .trim();
}

function mergeNativeButtons(snapshot, item, source) {
    if (!Array.isArray(item?.buttons)) return snapshot;
    const buttons = snapshot.buttons.map(button => ({ ...button }));
    for (const override of item.buttons) {
        const index = Number(override?.index);
        if (!Number.isInteger(index) || !buttons[index]) continue;
        const pressed = Boolean(override.pressed);
        buttons[index] = {
            ...buttons[index],
            pressed,
            touched: pressed || buttons[index].touched,
            value: pressed ? 1 : 0,
        };
    }
    return {
        ...snapshot,
        buttons,
        hardware: {
            name: String(item.name || ''),
            vendor: Number(item.vendor) || null,
            product: Number(item.product) || null,
        },
        connection: normalizeConnection(item.connection),
        nativeInput: String(source || 'native'),
    };
}

function normalizeConnection(connection) {
    const transport = String(connection?.transport || '').toLowerCase();
    if (!transport) return null;
    return {
        transport,
        signal: normalizeSignal(connection.signal),
        adapter: normalizeAdapter(connection.adapter),
    };
}

function normalizeSignal(signal) {
    const kind = String(signal?.kind || '');
    if (signal?.value === null || signal?.value === undefined) return null;
    const value = Number(signal?.value);
    const validAbsolute = kind === 'absolute_dbm' && value >= -127 && value <= 20;
    const validMargin = kind === 'bredr_link_margin_db' && value >= -128 && value <= 127;
    if ((!validAbsolute && !validMargin) || !Number.isFinite(value)) return null;
    return {
        kind,
        source: String(signal.source || ''),
        value,
    };
}

function normalizeAdapter(adapter) {
    const name = String(adapter?.name || '').trim();
    if (!name) return null;
    return {
        name,
        address: optionalString(adapter.address),
        vendor: optionalString(adapter.vendor),
        model: optionalString(adapter.model),
        hardwareId: optionalString(adapter.hardware_id),
        path: optionalString(adapter.path),
    };
}

function optionalString(value) {
    const text = String(value || '').trim();
    return text || null;
}

function finiteNumber(value) {
    const number = Number(value);
    return Number.isFinite(number) ? number : 0;
}

function signalStrength(kind, value) {
    if (!Number.isFinite(value)) return null;
    const floor = kind === 'bredr_link_margin_db' ? BREDR_MARGIN_FLOOR_DB : SIGNAL_FLOOR_DBM;
    const ceiling = kind === 'bredr_link_margin_db' ? 0 : SIGNAL_CEILING_DBM;
    const normalized = clamp(
        (value - floor) / (ceiling - floor),
        0,
        1,
    );
    return Math.round(SIGNAL_MINIMUM_HEIGHT + normalized * (100 - SIGNAL_MINIMUM_HEIGHT));
}

function signalRangeLabel(kind, minimum, maximum) {
    if (minimum === null || maximum === null) return 'Signal unavailable';
    if (kind !== 'bredr_link_margin_db') {
        if (minimum === maximum) return `${minimum} dBm`;
        return `${minimum} to ${maximum} dBm`;
    }
    if (minimum === maximum) return relativeValueLabel(minimum);
    if (maximum <= 0) return `${Math.abs(maximum)} to ${Math.abs(minimum)} dB below target`;
    if (minimum >= 0) return `${minimum} to ${maximum} dB above target`;
    return `${formatSigned(minimum)} to ${formatSigned(maximum)} dB relative`;
}

function relativeValueLabel(value) {
    if (value < 0) return `${Math.abs(value)} dB below target`;
    if (value > 0) return `${value} dB above target`;
    return 'Target range';
}

function clamp(value, minimum, maximum) {
    return Math.min(Math.max(value, minimum), maximum);
}
