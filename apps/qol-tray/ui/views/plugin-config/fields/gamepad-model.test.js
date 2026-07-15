import test from 'node:test';
import assert from 'node:assert/strict';
import {
    activeInputs,
    appendSignalHistory,
    chooseGamepad,
    connectionPresentation,
    gamepadSnapshot,
    mergeNativeInputs,
    monitorSignature,
    plotSignalHistory,
    signalHistorySample,
    signalHistorySummary,
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
            connection: {
                transport: 'bluetooth',
                signal: {
                    kind: 'bredr_link_margin_db',
                    source: 'hci_link',
                    value: -11,
                },
                adapter: {
                    name: 'hci7',
                    address: '00:11:22:33:44:55',
                    vendor: 'Foo Corp.',
                    model: 'Bar Radio',
                    hardware_id: '1234:abcd',
                    path: 'pci-0000:00:01.0-usb-0:2:1.0',
                },
            },
            buttons: [
                { index: 10, pressed: true },
                { index: 11, pressed: false },
            ],
        }],
    });

    assert.equal(merged[0].buttons[10].pressed, true);
    assert.equal(merged[0].buttons[11].pressed, false);
    assert.equal(merged[0].nativeInput, 'linux-evdev');
    assert.deepEqual(merged[0].connection, {
        transport: 'bluetooth',
        signal: {
            kind: 'bredr_link_margin_db',
            source: 'hci_link',
            value: -11,
        },
        adapter: {
            name: 'hci7',
            address: '00:11:22:33:44:55',
            vendor: 'Foo Corp.',
            model: 'Bar Radio',
            hardwareId: '1234:abcd',
            path: 'pci-0000:00:01.0-usb-0:2:1.0',
        },
    });
    assert.equal(merged[1].buttons[10].pressed, false);
    assert.equal(merged[1].nativeInput, null);
});

test('connection presentation grades absolute RSSI and leaves BR/EDR relative RSSI ungraded', () => {
    const cases = [
        [absoluteSignal(-40), ['Strong RSSI', 4, 'excellent', -40, '-40 dBm']],
        [absoluteSignal(-60), ['Usable RSSI', 3, 'good', -60, '-60 dBm']],
        [absoluteSignal(-72), ['Low RSSI', 2, 'fair', -72, '-72 dBm']],
        [absoluteSignal(-88), ['Very low RSSI', 1, 'weak', -88, '-88 dBm']],
        [relativeSignal(7), ['Connected', null, 'connected', 7, 'HCI +7 dB relative']],
        [relativeSignal(0), ['Connected', null, 'connected', 0, 'HCI 0 dB relative']],
        [relativeSignal(-3), ['Connected', null, 'connected', -3, 'HCI -3 dB relative']],
        [relativeSignal(-11), ['Connected', null, 'connected', -11, 'HCI -11 dB relative']],
        [{ transport: 'bluetooth', signal: null }, ['Connected', null, 'connected', null, null]],
        [{ transport: 'usb', signal: null }, ['Wired', null, 'wired', null, null]],
    ];
    for (const [connection, expected] of cases) {
        const result = connectionPresentation(connection);
        assert.deepEqual(
            [result.detail, result.level, result.tone, result.signalValue, result.valueLabel],
            expected,
            JSON.stringify(connection),
        );
    }
    assert.match(connectionPresentation(relativeSignal(-11)).label, /ungraded telemetry/);
    assert.match(connectionPresentation(absoluteSignal(-40)).label, /does not measure packet delivery/);
    assert.equal(connectionPresentation(null), null);
});

test('signal history only assigns absolute measurements to a fixed quality scale', () => {
    const cases = [
        [absoluteSignal(-35), false, [-35, 'absolute_dbm', 100, 'excellent']],
        [absoluteSignal(-65), false, [-65, 'absolute_dbm', 56, 'good']],
        [absoluteSignal(-95), false, [-95, 'absolute_dbm', 12, 'weak']],
        [relativeSignal(0), false, [0, 'bredr_link_margin_db', null, 'neutral']],
        [relativeSignal(-10), false, [-10, 'bredr_link_margin_db', null, 'neutral']],
        [relativeSignal(-20), false, [-20, 'bredr_link_margin_db', null, 'neutral']],
        [{ transport: 'bluetooth', signal: null }, false, [null, null, null, 'neutral']],
        [null, true, [null, null, null, 'neutral']],
    ];

    for (const [connection, bluetoothKnown, expected] of cases) {
        const sample = signalHistorySample(connection, bluetoothKnown);
        assert.deepEqual(
            [sample.value, sample.kind, sample.strength, sample.tone],
            expected,
            JSON.stringify({ connection, bluetoothKnown }),
        );
    }
    assert.equal(signalHistorySample(null, false), null);
    assert.equal(signalHistorySample({ transport: 'usb', signal: null }), null);
});

test('relative RSSI history plots variation inside its own window without grading quality', () => {
    const relative = [-10, -6, -2].map(value => signalHistorySample(relativeSignal(value)));
    const plotted = plotSignalHistory(relative);

    assert.deepEqual(plotted.map(sample => sample.strength), [12, 56, 100]);
    assert.deepEqual(plotted.map(sample => sample.value), [-10, -6, -2]);
    assert.deepEqual(
        plotSignalHistory([signalHistorySample(relativeSignal(-6))])
            .map(sample => sample.strength),
        [56],
    );

    const unavailable = signalHistorySample({ transport: 'bluetooth', signal: null });
    assert.equal(plotSignalHistory([relative[0], unavailable])[1].strength, null);

    const absolute = signalHistorySample(absoluteSignal(-65));
    assert.strictEqual(plotSignalHistory([absolute])[0], absolute);
});

test('signal history trims oldest samples and summarizes range and unavailable gaps', () => {
    const samples = [
        signalHistorySample(absoluteSignal(-72)),
        signalHistorySample({ transport: 'bluetooth', signal: null }),
        signalHistorySample(absoluteSignal(-58)),
        signalHistorySample(absoluteSignal(-64)),
    ];
    const history = samples.reduce(
        (current, sample) => appendSignalHistory(current, sample, 3),
        [],
    );

    assert.deepEqual(history, samples.slice(1));
    assert.deepEqual(signalHistorySummary(history), {
        count: 3,
        kind: 'absolute_dbm',
        title: 'RSSI history · 60 s',
        unavailableCount: 1,
        minimum: -64,
        maximum: -58,
        rangeLabel: '-64 to -58 dBm',
        gapLabel: '1 unavailable',
        label: 'Bluetooth link history: 3 samples, -64 to -58 dBm, 1 measurement unavailable',
    });
    assert.equal(
        signalHistorySummary([samples[0], samples[2]]).label,
        'Bluetooth link history: 2 samples, -72 to -58 dBm, no measurements unavailable',
    );
    assert.deepEqual(
        signalHistorySummary([samples[1], samples[1]]),
        {
            count: 2,
            kind: null,
            title: 'RSSI history · 60 s',
            unavailableCount: 2,
            minimum: null,
            maximum: null,
            rangeLabel: 'Signal unavailable',
            gapLabel: '2 unavailable',
            label: 'Bluetooth link history: 2 samples, Signal unavailable, 2 measurements unavailable',
        },
    );
    const relative = [
        signalHistorySample(relativeSignal(-11)),
        signalHistorySample(relativeSignal(-2)),
    ];
    assert.equal(signalHistorySummary(relative).rangeLabel, '-11 to -2 dB relative');
    assert.equal(signalHistorySummary(relative).title, 'Relative RSSI trend · 60 s · ungraded');
    assert.match(signalHistorySummary(relative).label, /not a connection quality grade/);
    assert.equal(signalHistorySummary([]), null);
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
    const signalChanged = { ...neutral, connection: absoluteSignal(-70) };

    assert.notEqual(monitorSignature([neutral], neutral), monitorSignature([pressed], pressed));
    assert.notEqual(monitorSignature([neutral], neutral), monitorSignature([moved], moved));
    assert.notEqual(
        monitorSignature([neutral], neutral),
        monitorSignature([signalChanged], signalChanged),
    );
    assert.notEqual(monitorSignature([neutral], neutral), monitorSignature([neutral, moved], neutral));
});

function absoluteSignal(value) {
    return {
        transport: 'bluetooth',
        signal: { kind: 'absolute_dbm', source: 'bluez_device', value },
    };
}

function relativeSignal(value) {
    return {
        transport: 'bluetooth',
        signal: { kind: 'bredr_link_margin_db', source: 'hci_link', value },
    };
}
