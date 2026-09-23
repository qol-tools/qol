import test from 'node:test';
import assert from 'node:assert/strict';
import {
    HAPTIC_MODE_DUAL_RUMBLE,
    HAPTIC_MODE_PULSE,
    hapticCapability,
    rumbleSteps,
} from './gamepad-haptics.js';

test('haptic capability detects standards, legacy dual rumble, and pulse actuators', () => {
    const playEffect = () => {};
    const cases = [
        [null, null],
        [{ vibrationActuator: { effects: ['dual-rumble'], playEffect } }, HAPTIC_MODE_DUAL_RUMBLE],
        [{ vibrationActuator: { type: 'dual-rumble', playEffect } }, HAPTIC_MODE_DUAL_RUMBLE],
        [{ hapticActuators: [{ pulse() {} }] }, HAPTIC_MODE_PULSE],
        [{ vibrationActuator: { effects: [], playEffect } }, null],
    ];
    for (const [gamepad, expected] of cases) {
        assert.equal(hapticCapability(gamepad).mode, expected, JSON.stringify(gamepad));
    }
});

test('dual rumble patterns clamp magnitudes and keep every effect short', () => {
    assert.deepEqual(rumbleSteps('low', 120, 80, HAPTIC_MODE_DUAL_RUMBLE), [{
        duration: 800,
        strongMagnitude: 1,
        weakMagnitude: 0,
    }]);
    assert.deepEqual(rumbleSteps('high', 80, -20, HAPTIC_MODE_DUAL_RUMBLE), [{
        duration: 800,
        strongMagnitude: 0,
        weakMagnitude: 0,
    }]);
    assert.deepEqual(rumbleSteps('both', 60, 40, HAPTIC_MODE_DUAL_RUMBLE), [{
        duration: 800,
        strongMagnitude: 0.6,
        weakMagnitude: 0.4,
    }]);
    const sweep = rumbleSteps('sweep', 100, 100, HAPTIC_MODE_DUAL_RUMBLE);
    assert.equal(sweep.length, 5);
    assert.ok(sweep.every(step => step.duration === 240));
    assert.deepEqual(sweep.at(-1), {
        duration: 240,
        strongMagnitude: 1,
        weakMagnitude: 1,
    });
});

test('pulse fallback collapses dual motor patterns to one actuator', () => {
    assert.deepEqual(rumbleSteps('both', 35, 70, HAPTIC_MODE_PULSE), [{
        duration: 800,
        intensity: 0.7,
    }]);
    const sweep = rumbleSteps('sweep', 100, 50, HAPTIC_MODE_PULSE);
    assert.deepEqual(sweep.map(step => step.intensity), [1, 0.65, 0.35, 0.5, 1]);
});
