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
        haptics: Boolean(gamepad.vibrationActuator),
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
    const connection = selected.connection
        ? `${selected.connection.transport}:${selected.connection.signalDbm ?? ''}`
        : '';
    return `${inventory}/${selected.index}/${selected.timestamp}/${selected.mappingProfile || ''}/${selected.nativeInput || ''}/${connection}/${buttons}/${axes}`;
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
            signalDbm: null,
            label: 'USB wired connection',
        };
    }
    if (transport !== 'bluetooth') {
        return {
            transport: 'Controller',
            detail: 'Connected',
            level: null,
            tone: 'neutral',
            signalDbm: null,
            label: 'Controller connected',
        };
    }
    const hasSignal = connection.signalDbm !== null && connection.signalDbm !== undefined;
    const signalDbm = hasSignal ? Number(connection.signalDbm) : Number.NaN;
    if (!Number.isFinite(signalDbm) || signalDbm < -127 || signalDbm > 20) {
        return {
            transport: 'Bluetooth',
            detail: 'Signal unavailable',
            level: 0,
            tone: 'neutral',
            signalDbm: null,
            label: 'Bluetooth signal unavailable',
        };
    }
    const rounded = Math.round(signalDbm);
    const [detail, level, tone] = rounded >= -55
        ? ['Excellent', 4, 'excellent']
        : rounded >= -67
            ? ['Good', 3, 'good']
            : rounded >= -78
                ? ['Fair', 2, 'fair']
                : ['Weak', 1, 'weak'];
    return {
        transport: 'Bluetooth',
        detail,
        level,
        tone,
        signalDbm: rounded,
        label: `Bluetooth signal ${detail.toLowerCase()}, ${rounded} dBm`,
    };
}

export function signalHistorySample(connection, bluetoothKnown = false) {
    const signal = connectionPresentation(connection);
    if (signal?.transport === 'Bluetooth') {
        return {
            signalDbm: signal.signalDbm,
            strength: signalStrength(signal.signalDbm),
            tone: signal.tone,
        };
    }
    if (!connection && bluetoothKnown) {
        return { signalDbm: null, strength: null, tone: 'neutral' };
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
    const readings = history
        .map(sample => Number(sample?.signalDbm))
        .filter((value, index) => history[index]?.signalDbm !== null && Number.isFinite(value));
    const unavailableCount = history.length - readings.length;
    const minimumDbm = readings.length > 0 ? Math.min(...readings) : null;
    const maximumDbm = readings.length > 0 ? Math.max(...readings) : null;
    const rangeLabel = signalRangeLabel(minimumDbm, maximumDbm);
    const gapLabel = unavailableCount === 0 ? 'No gaps' : `${unavailableCount} unavailable`;
    const unavailableLabel = unavailableCount === 0
        ? 'no measurements unavailable'
        : unavailableCount === 1
            ? '1 measurement unavailable'
            : `${unavailableCount} measurements unavailable`;
    return {
        count: history.length,
        unavailableCount,
        minimumDbm,
        maximumDbm,
        rangeLabel,
        gapLabel,
        label: `Bluetooth signal history: ${history.length} samples, ${rangeLabel}, ${unavailableLabel}`,
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
    const signalDbm = Number(connection.signal_dbm);
    return {
        transport,
        signalDbm: connection.signal_dbm !== null
            && connection.signal_dbm !== undefined
            && Number.isFinite(signalDbm)
            ? signalDbm
            : null,
    };
}

function finiteNumber(value) {
    const number = Number(value);
    return Number.isFinite(number) ? number : 0;
}

function signalStrength(signalDbm) {
    if (!Number.isFinite(signalDbm)) return null;
    const normalized = clamp(
        (signalDbm - SIGNAL_FLOOR_DBM) / (SIGNAL_CEILING_DBM - SIGNAL_FLOOR_DBM),
        0,
        1,
    );
    return Math.round(SIGNAL_MINIMUM_HEIGHT + normalized * (100 - SIGNAL_MINIMUM_HEIGHT));
}

function signalRangeLabel(minimumDbm, maximumDbm) {
    if (minimumDbm === null || maximumDbm === null) return 'RSSI unavailable';
    if (minimumDbm === maximumDbm) return `${minimumDbm} dBm`;
    return `${minimumDbm} to ${maximumDbm} dBm`;
}

function clamp(value, minimum, maximum) {
    return Math.min(Math.max(value, minimum), maximum);
}
