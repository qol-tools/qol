import { html } from '../../../lib/html.js';
import { useCallback, useState } from 'preact/hooks';
import { Button } from '../../../lib/components/Button.js';
import { Slider } from '../../../lib/components/Slider.js';
import { toast } from '../../../lib/toast.js';
import { HAPTIC_MODE_PULSE } from './gamepad-haptics.js';
import { useGamepadRumble } from './useGamepadRumble.js';

export function GamepadRumbleLab({ snapshot }) {
    const pulseOnly = snapshot.hapticMode === HAPTIC_MODE_PULSE;
    const [low, setLow] = useState(75);
    const [high, setHigh] = useState(55);
    const rumble = useGamepadRumble(snapshot.index);
    const run = useCallback(async pattern => {
        const error = await rumble.play(pattern, low, pulseOnly ? low : high);
        if (error) toast('error', `Rumble test failed: ${error}`);
    }, [high, low, pulseOnly, rumble.play]);
    const stop = useCallback(() => {
        rumble.stop().catch(error => {
            toast('error', `Could not stop controller rumble: ${error.message || error}`);
        });
    }, [rumble.stop]);
    return html`
        <section class="gamepad-rumble" aria-label="Rumble laboratory">
            <div class="gamepad-rumble-heading">
                <div>
                    <strong>Rumble lab</strong>
                    <span>${pulseOnly
                        ? 'This browser exposes one combined haptic actuator.'
                        : 'Test the low-frequency and high-frequency motors independently.'}</span>
                </div>
                <output role="status" aria-live="polite">${rumbleStatus(rumble.activePattern)}</output>
            </div>
            <div class="gamepad-rumble-sliders" data-single=${pulseOnly}>
                <${Slider}
                    label=${pulseOnly ? 'Intensity' : 'Low-frequency motor'}
                    description=${pulseOnly ? '' : 'Heavy, lower-pitched vibration'}
                    value=${low}
                    min=${0}
                    max=${100}
                    step=${5}
                    unit="%"
                    onInput=${setLow} />
                ${!pulseOnly && html`
                    <${Slider}
                        label="High-frequency motor"
                        description="Sharper, higher-pitched vibration"
                        value=${high}
                        min=${0}
                        max=${100}
                        step=${5}
                        unit="%"
                        onInput=${setHigh} />
                `}
            </div>
            <div class="gamepad-rumble-actions">
                ${!pulseOnly && html`
                    <${RumbleButton} label="Test low" pattern="low" rumble=${rumble} run=${run} />
                    <${RumbleButton} label="Test high" pattern="high" rumble=${rumble} run=${run} />
                `}
                <${RumbleButton} label=${pulseOnly ? 'Test rumble' : 'Test both'}
                    pattern="both" primary=${true} rumble=${rumble} run=${run} />
                ${!pulseOnly && html`
                    <${RumbleButton} label="Sweep" pattern="sweep" rumble=${rumble} run=${run} />
                `}
                <${Button} small=${true} variant="btn-outline-danger" onActivate=${stop}>Stop<//>
            </div>
            <span class="gamepad-rumble-safety">Every effect is duration-limited and stops when this page hides or the controller disconnects.</span>
        </section>
    `;
}

function RumbleButton({ label, pattern, primary = false, rumble, run }) {
    const active = rumble.activePattern === pattern;
    return html`
        <${Button} small=${true} variant=${primary ? 'btn-primary' : 'btn-ghost'}
            className=${active ? 'is-active' : ''}
            aria-pressed=${active}
            onActivate=${() => run(pattern)}>${label}<//>
    `;
}

function rumbleStatus(pattern) {
    if (pattern === 'low') return 'Testing low frequency';
    if (pattern === 'high') return 'Testing high frequency';
    if (pattern === 'both') return 'Testing combined rumble';
    if (pattern === 'sweep') return 'Running sweep';
    return 'Ready';
}
