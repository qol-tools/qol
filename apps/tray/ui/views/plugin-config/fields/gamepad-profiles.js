const STANDARD_BUTTON_INDICES = Object.freeze(Array.from({ length: 17 }, (_, index) => index));

const XBOX_OFFSET_LAYOUT = Object.freeze({
    leftStick: Object.freeze({ x: 220, y: 188 }),
    dpad: Object.freeze({ x: 305, y: 287 }),
    face: Object.freeze({ x: 590, y: 204 }),
    rightStick: Object.freeze({ x: 510, y: 290 }),
});

const PLAYSTATION_SYMMETRIC_LAYOUT = Object.freeze({
    leftStick: Object.freeze({ x: 300, y: 315 }),
    dpad: Object.freeze({ x: 190, y: 198 }),
    face: Object.freeze({ x: 610, y: 198 }),
    rightStick: Object.freeze({ x: 500, y: 315 }),
});

const XBOX_FACE = Object.freeze([
    Object.freeze({ label: 'Y', index: 3, dx: 0, dy: -40, tone: 'warning' }),
    Object.freeze({ label: 'B', index: 1, dx: 40, dy: 0, tone: 'danger' }),
    Object.freeze({ label: 'A', index: 0, dx: 0, dy: 40, tone: 'success' }),
    Object.freeze({ label: 'X', index: 2, dx: -40, dy: 0, tone: 'blue' }),
]);

const PLAYSTATION_FACE = Object.freeze([
    Object.freeze({ label: '△', index: 3, dx: 0, dy: -40, tone: 'warning' }),
    Object.freeze({ label: '○', index: 1, dx: 40, dy: 0, tone: 'danger' }),
    Object.freeze({ label: '×', index: 0, dx: 0, dy: 40, tone: 'success' }),
    Object.freeze({ label: '□', index: 2, dx: -40, dy: 0, tone: 'accent' }),
]);

const SWITCH_FACE = Object.freeze([
    Object.freeze({ label: 'X', index: 3, dx: 0, dy: -40, tone: 'warning' }),
    Object.freeze({ label: 'A', index: 1, dx: 40, dy: 0, tone: 'danger' }),
    Object.freeze({ label: 'B', index: 0, dx: 0, dy: 40, tone: 'success' }),
    Object.freeze({ label: 'Y', index: 2, dx: -40, dy: 0, tone: 'accent' }),
]);

const GULIKIT_DEVICE_CONTROLS = Object.freeze([
    Object.freeze({ id: 'screenshot', label: 'Screenshot', shortLabel: 'CAP', x: 350, y: 214 }),
    Object.freeze({ id: 'setting', label: 'Setting', shortLabel: 'SET', x: 400, y: 214 }),
    Object.freeze({ id: 'apg', label: 'APG', shortLabel: 'APG', x: 450, y: 214 }),
    Object.freeze({ id: 'mode', label: 'Mode / power', shortLabel: 'M', x: 400, y: 254 }),
]);

const PROFILES = Object.freeze([
    Object.freeze({
        id: 'gulikit-kingkong-2-pro',
        label: 'GuliKit KingKong 2 Pro',
        ariaLabel: 'Live GuliKit KingKong 2 Pro controller input diagram',
        standardMapping: true,
        claimedButtons: STANDARD_BUTTON_INDICES,
        layout: XBOX_OFFSET_LAYOUT,
        faceButtons: XBOX_FACE,
        centerVariant: 'gulikit',
        shoulderLabels: Object.freeze(['L', 'R']),
        triggerLabels: Object.freeze(['ZL', 'ZR']),
        deviceControls: GULIKIT_DEVICE_CONTROLS,
        deviceNote: 'APG, Setting, and Screenshot are controller-side functions in PC XInput mode and emit no testable button event. Screenshot is exposed in Switch mode.',
        matches: identity => identity.includes('gulikit controller xw')
            || identity.includes('gulikit kingkong 2'),
    }),
    Object.freeze({
        id: 'playstation',
        label: 'PlayStation-style',
        ariaLabel: 'Live PlayStation-style controller input diagram',
        standardMapping: true,
        claimedButtons: STANDARD_BUTTON_INDICES,
        layout: PLAYSTATION_SYMMETRIC_LAYOUT,
        faceButtons: PLAYSTATION_FACE,
        centerVariant: 'playstation',
        shoulderLabels: Object.freeze(['L1', 'R1']),
        triggerLabels: Object.freeze(['L2', 'R2']),
        deviceControls: Object.freeze([]),
        deviceNote: null,
        matches: identity => identity.includes('dualsense')
            || identity.includes('dualshock')
            || identity.includes('054c'),
    }),
    Object.freeze({
        id: 'switch-pro',
        label: 'Nintendo Switch Pro-style',
        ariaLabel: 'Live Nintendo Switch Pro-style controller input diagram',
        standardMapping: true,
        claimedButtons: STANDARD_BUTTON_INDICES,
        layout: XBOX_OFFSET_LAYOUT,
        faceButtons: SWITCH_FACE,
        centerVariant: 'switch',
        shoulderLabels: Object.freeze(['L', 'R']),
        triggerLabels: Object.freeze(['ZL', 'ZR']),
        deviceControls: Object.freeze([]),
        deviceNote: null,
        matches: identity => identity.includes('057e')
            || identity.includes('nintendo switch')
            || identity.includes('switch pro'),
    }),
]);

const XBOX_PROFILE = Object.freeze({
    id: 'xbox-standard',
    label: 'Xbox-style',
    ariaLabel: 'Live Xbox-style controller input diagram',
    standardMapping: true,
    claimedButtons: STANDARD_BUTTON_INDICES,
    layout: XBOX_OFFSET_LAYOUT,
    faceButtons: XBOX_FACE,
    centerVariant: 'xbox',
    shoulderLabels: Object.freeze(['LB', 'RB']),
    triggerLabels: Object.freeze(['LT', 'RT']),
    deviceControls: Object.freeze([]),
    deviceNote: null,
});

const RAW_PROFILE = Object.freeze({
    id: 'generic-raw',
    label: 'Generic raw controller',
    ariaLabel: 'Live generic controller input diagram with automatically discovered controls',
    standardMapping: false,
    claimedButtons: Object.freeze([]),
    layout: XBOX_OFFSET_LAYOUT,
    faceButtons: XBOX_FACE,
    centerVariant: 'xbox',
    shoulderLabels: Object.freeze(['L1', 'R1']),
    triggerLabels: Object.freeze(['L2', 'R2']),
    deviceControls: Object.freeze([]),
    deviceNote: 'This controller has no standard browser mapping. Every exposed button is added to the discovery rail instead of guessing its physical position.',
});

export function controllerProfile(snapshot) {
    if (!snapshot) return XBOX_PROFILE;
    if (snapshot.mapping !== 'standard') return RAW_PROFILE;
    const identity = controllerIdentity(snapshot);
    const matched = PROFILES.find(profile => profile.matches(identity));
    if (matched) return matched;
    return XBOX_PROFILE;
}

export function unmappedProfileButtons(snapshot, profile = controllerProfile(snapshot)) {
    if (!snapshot?.buttons?.length) return [];
    const claimed = new Set(profile.claimedButtons);
    return snapshot.buttons.filter(button => !claimed.has(button.index));
}

function controllerIdentity(snapshot) {
    return [
        snapshot.id,
        snapshot.hardware?.name,
        formatHardwareId(snapshot.hardware?.vendor),
        formatHardwareId(snapshot.hardware?.product),
    ]
        .filter(Boolean)
        .join(' ')
        .toLowerCase();
}

function formatHardwareId(value) {
    const number = Number(value);
    return Number.isInteger(number) ? number.toString(16).padStart(4, '0') : '';
}
