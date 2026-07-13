import test from 'node:test';
import assert from 'node:assert/strict';
import {
    activeInputs,
    chooseGamepad,
    connectionPresentation,
    gamepadSnapshot,
    mergeNativeInputs,
    monitorSignature,
} from './gamepad-model.js';

function pad({
    id,
    index = 0,
    timestamp = 1,
    mapping = 'standard',
    axes = [0, 0, 0, 0],
    buttons = [],
    buttonCount = Math.max(buttons.length, 17),
} = {}) {
    return gamepadSnapshot({
        id: id || `Pad ${index}`,
        index,
        connected: true,
        timestamp,
        mapping,
        axes,
        buttons: Array.from({ length: buttonCount }, (_, buttonIndex) => ({
            pressed: Boolean(buttons[buttonIndex]?.pressed),
            touched: Boolean(buttons[buttonIndex]?.touched),
            value: buttons[buttonIndex]?.value || 0,
        })),
    });
}

test('standard gamepad snapshot maps canonical controls and clamps values', () => {
    const snapshot = pad({
        axes: [-2, 0.25, 2, 0],
        buttons: [{ pressed: true, touched: true, value: 1.5 }],
    });

    assert.equal(snapshot.mapping, 'standard');
    assert.equal(snapshot.buttons[0].name, 'A');
    assert.deepEqual(snapshot.axes.map(axis => axis.name), ['Left X', 'Left Y', 'Right X', 'Right Y']);
    assert.deepEqual(snapshot.axes.map(axis => axis.value), [-1, 0.25, 1, 0]);
    assert.deepEqual(snapshot.buttons[0], {
        index: 0,
        name: 'A',
        value: 1,
        pressed: true,
        touched: true,
    });
});

test('raw gamepad snapshot keeps index-based labels', () => {
    const snapshot = pad({ mapping: '', axes: [0.4], buttons: [{ value: 0.3 }] });

    assert.equal(snapshot.mapping, 'raw');
    assert.equal(snapshot.axes[0].name, 'A0');
    assert.equal(snapshot.buttons[0].name, 'B0');
});

test('GuliKit Firefox layout maps every exposed physical button to its canonical control', () => {
    const cases = [
        [0, 0],
        [1, 1],
        [18, 2],
        [3, 3],
        [2, 4],
        [19, 5],
        [6, 6],
        [7, 7],
        [4, 8],
        [5, 9],
        [12, 12],
        [13, 13],
        [14, 14],
        [15, 15],
        [17, 16],
    ];

    for (const [rawIndex, expectedIndex] of cases) {
        const buttons = Array.from({ length: 20 }, () => ({ value: 0 }));
        buttons[rawIndex] = { pressed: true, touched: true, value: 1 };
        const snapshot = pad({
            id: '045e-02e0-GuliKit Controller XW',
            buttons,
            buttonCount: 20,
        });

        assert.deepEqual(
            snapshot.buttons.filter(button => button.pressed).map(button => button.index),
            [expectedIndex],
            `raw button ${rawIndex}`,
        );
        assert.equal(snapshot.buttons.length, 17, `raw button ${rawIndex}`);
        assert.equal(snapshot.mappingProfile, 'GuliKit Firefox correction', `raw button ${rawIndex}`);
    }
});

test('GuliKit correction leaves already-standard Chromium layouts alone', () => {
    const snapshot = pad({
        id: '045e-02e0-GuliKit Controller XW',
        buttons: [{ value: 0 }, { value: 0 }, { pressed: true, value: 1 }],
        buttonCount: 17,
    });

    assert.equal(snapshot.mappingProfile, null);
    assert.equal(snapshot.buttons[2].pressed, true);
});

test('native input supplements matching standard pads without leaking across devices', () => {
    const gulikit = pad({ id: '045e-02e0-GuliKit Controller XW' });
    const other = pad({ id: 'Other pad', index: 1 });
    const merged = mergeNativeInputs([gulikit, other], {
        source: 'linux-evdev',
        items: [{
            name: 'GuliKit Controller XW',
            connection: { transport: 'bluetooth', signal_dbm: -58 },
            buttons: [
                { index: 10, pressed: true },
                { index: 11, pressed: false },
            ],
        }],
    });

    assert.equal(merged[0].buttons[10].pressed, true);
    assert.equal(merged[0].buttons[11].pressed, false);
    assert.equal(merged[0].nativeInput, 'linux-evdev');
    assert.deepEqual(merged[0].connection, { transport: 'bluetooth', signalDbm: -58 });
    assert.equal(merged[1].buttons[10].pressed, false);
    assert.equal(merged[1].nativeInput, null);
});

test('connection presentation distinguishes real signal, unavailable signal, and wired pads', () => {
    const cases = [
        [{ transport: 'bluetooth', signalDbm: -40 }, ['Excellent', 4, 'excellent', -40]],
        [{ transport: 'bluetooth', signalDbm: -60 }, ['Good', 3, 'good', -60]],
        [{ transport: 'bluetooth', signalDbm: -72 }, ['Fair', 2, 'fair', -72]],
        [{ transport: 'bluetooth', signalDbm: -88 }, ['Weak', 1, 'weak', -88]],
        [{ transport: 'bluetooth', signalDbm: null }, ['Signal unavailable', 0, 'neutral', null]],
        [{ transport: 'usb', signalDbm: null }, ['Wired', null, 'wired', null]],
    ];
    for (const [connection, expected] of cases) {
        const result = connectionPresentation(connection);
        assert.deepEqual(
            [result.detail, result.level, result.tone, result.signalDbm],
            expected,
            JSON.stringify(connection),
        );
    }
    assert.equal(connectionPresentation(null), null);
});

test('controller selection honors preference and otherwise follows latest activity', () => {
    const first = pad({ index: 0, timestamp: 8 });
    const latest = pad({ index: 2, timestamp: 12 });

    assert.equal(chooseGamepad([first, latest], 'auto').index, 2);
    assert.equal(chooseGamepad([first, latest], '0').index, 0);
    assert.equal(chooseGamepad([first, latest], '9').index, 2);
    assert.equal(chooseGamepad([], 'auto'), null);
});

test('active input summary applies deadzone and names analog input values', () => {
    const snapshot = pad({
        axes: [0.05, -0.5, 0, 0],
        buttons: [{ value: 0.4 }, { pressed: true, value: 1 }],
    });

    assert.deepEqual(activeInputs(snapshot), ['A 0.40', 'B', 'Left Y -0.50']);
});

test('monitor signature changes for inventory, connection, buttons, and axes', () => {
    const neutral = pad();
    const pressed = pad({ buttons: [{ pressed: true, value: 1 }] });
    const moved = pad({ axes: [0.5, 0, 0, 0] });
    const signalChanged = { ...neutral, connection: { transport: 'bluetooth', signalDbm: -70 } };

    assert.notEqual(monitorSignature([neutral], neutral), monitorSignature([pressed], pressed));
    assert.notEqual(monitorSignature([neutral], neutral), monitorSignature([moved], moved));
    assert.notEqual(
        monitorSignature([neutral], neutral),
        monitorSignature([signalChanged], signalChanged),
    );
    assert.notEqual(monitorSignature([neutral], neutral), monitorSignature([neutral, moved], neutral));
});
