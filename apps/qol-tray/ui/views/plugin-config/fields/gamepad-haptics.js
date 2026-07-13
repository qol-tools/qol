export const HAPTIC_MODE_DUAL_RUMBLE = 'dual-rumble';
export const HAPTIC_MODE_PULSE = 'pulse';

const TEST_DURATION_MS = 800;
const SWEEP_STEP_DURATION_MS = 240;

export function hapticCapability(gamepad) {
    const actuator = gamepad?.vibrationActuator || gamepad?.hapticActuators?.[0] || null;
    if (!actuator) return { actuator: null, mode: null, effects: [] };
    const effects = Array.from(actuator.effects || [], String);
    const dualRumble = effects.includes(HAPTIC_MODE_DUAL_RUMBLE)
        || actuator.type === HAPTIC_MODE_DUAL_RUMBLE;
    if (dualRumble && typeof actuator.playEffect === 'function') {
        return { actuator, mode: HAPTIC_MODE_DUAL_RUMBLE, effects };
    }
    if (typeof actuator.pulse === 'function') {
        return { actuator, mode: HAPTIC_MODE_PULSE, effects };
    }
    return { actuator, mode: null, effects };
}

export function rumbleSteps(pattern, lowPercent, highPercent, mode) {
    const low = normalizeMagnitude(lowPercent);
    const high = normalizeMagnitude(highPercent);
    const dualSteps = dualRumbleSteps(pattern, low, high);
    if (mode !== HAPTIC_MODE_PULSE) return dualSteps;
    return dualSteps.map(step => ({
        duration: step.duration,
        intensity: Math.max(step.strongMagnitude, step.weakMagnitude),
    }));
}

function dualRumbleSteps(pattern, low, high) {
    if (pattern === 'low') return [dualStep(TEST_DURATION_MS, low, 0)];
    if (pattern === 'high') return [dualStep(TEST_DURATION_MS, 0, high)];
    if (pattern === 'sweep') {
        return [
            dualStep(SWEEP_STEP_DURATION_MS, low, 0),
            dualStep(SWEEP_STEP_DURATION_MS, low * 0.65, high * 0.35),
            dualStep(SWEEP_STEP_DURATION_MS, low * 0.3, high * 0.7),
            dualStep(SWEEP_STEP_DURATION_MS, 0, high),
            dualStep(SWEEP_STEP_DURATION_MS, low, high),
        ];
    }
    return [dualStep(TEST_DURATION_MS, low, high)];
}

function dualStep(duration, strongMagnitude, weakMagnitude) {
    return {
        duration,
        strongMagnitude: clamp(strongMagnitude, 0, 1),
        weakMagnitude: clamp(weakMagnitude, 0, 1),
    };
}

function normalizeMagnitude(percent) {
    const value = Number(percent);
    if (!Number.isFinite(value)) return 0;
    return clamp(value / 100, 0, 1);
}

function clamp(value, minimum, maximum) {
    return Math.min(Math.max(value, minimum), maximum);
}
