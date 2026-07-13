import test from 'node:test';
import assert from 'node:assert/strict';
import {
    controllerProfile,
    unmappedProfileButtons,
} from './gamepad-profiles.js';
import { wellClearsGamepadCutout } from '../../../assets/gamepad-geometry.js';

function snapshot({ id = 'Generic pad', mapping = 'standard', buttonCount = 17 } = {}) {
    return {
        id,
        mapping,
        buttons: Array.from({ length: buttonCount }, (_, index) => ({
            index,
            name: mapping === 'standard' && index === 0 ? 'A' : `B${index}`,
            pressed: false,
            value: 0,
        })),
    };
}

test('GuliKit XInput identity selects its hardware profile and firmware controls', () => {
    const profile = controllerProfile(snapshot({
        id: '045e-02e0-GuliKit Controller XW',
    }));

    assert.equal(profile.id, 'gulikit-kingkong-2-pro');
    assert.deepEqual(
        profile.deviceControls.map(control => control.label),
        ['Screenshot', 'Setting', 'APG', 'Mode / power'],
    );
    assert.match(profile.deviceNote, /emit no testable button event/);
});

test('controller families select different geometry and legends', () => {
    const playstation = controllerProfile(snapshot({ id: '054c-0ce6-DualSense Wireless Controller' }));
    const switchPro = controllerProfile(snapshot({ id: '057e-2009-Nintendo Switch Pro Controller' }));
    const xbox = controllerProfile(snapshot({ id: 'Xbox Wireless Controller' }));

    assert.equal(playstation.id, 'playstation');
    assert.equal(playstation.layout.leftStick.y, playstation.layout.rightStick.y);
    assert.deepEqual(playstation.faceButtons.map(button => button.label), ['△', '○', '×', '□']);
    assert.equal(switchPro.id, 'switch-pro');
    assert.deepEqual(switchPro.faceButtons.map(button => button.label), ['X', 'A', 'B', 'Y']);
    assert.equal(xbox.id, 'xbox-standard');
    assert.equal(xbox.faceButtons.find(button => button.label === 'X')?.tone, 'blue');
});

test('offset layout clears the controller cutout and neighboring controls', () => {
    const profile = controllerProfile(snapshot({
        id: '045e-02e0-GuliKit Controller XW',
    }));
    const dpad = { ...profile.layout.dpad, radius: 58 };
    const rightStick = { ...profile.layout.rightStick, radius: 57 };
    const leftStick = { ...profile.layout.leftStick, radius: 57 };
    const capture = { ...profile.deviceControls[0], radius: 15 };
    const faceControls = Object.fromEntries(profile.faceButtons.map(control => [
        control.label,
        {
            x: profile.layout.face.x + control.dx,
            y: profile.layout.face.y + control.dy,
            radius: 23,
        },
    ]));

    assert.equal(wellClearsGamepadCutout(dpad, 8), true);
    assert.equal(wellClearsGamepadCutout(rightStick, 8), true);
    const clearances = [
        ['D-pad to left stick', dpad, leftStick],
        ['D-pad to capture', dpad, capture],
        ['right stick to A', rightStick, faceControls.A],
        ['right stick to X', rightStick, faceControls.X],
    ];
    for (const [label, first, second] of clearances) {
        const gap = Math.hypot(first.x - second.x, first.y - second.y)
            - first.radius
            - second.radius;
        assert.ok(gap >= 10, `${label} gap was ${gap}`);
    }
});

test('unknown standard extras are automatically discovered without moving canonical inputs', () => {
    const pad = snapshot({ buttonCount: 20 });
    const profile = controllerProfile(pad);

    assert.deepEqual(
        unmappedProfileButtons(pad, profile).map(button => button.index),
        [17, 18, 19],
    );
});

test('raw controllers put every exposed button on the discovery rail', () => {
    const pad = snapshot({
        id: '054c-0ce6-DualSense Wireless Controller',
        mapping: 'raw',
        buttonCount: 3,
    });
    const profile = controllerProfile(pad);

    assert.equal(profile.id, 'generic-raw');
    assert.deepEqual(
        unmappedProfileButtons(pad, profile).map(button => button.index),
        [0, 1, 2],
    );
});
