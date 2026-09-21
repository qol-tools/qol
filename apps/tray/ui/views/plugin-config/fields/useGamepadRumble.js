import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import {
    HAPTIC_MODE_DUAL_RUMBLE,
    HAPTIC_MODE_PULSE,
    hapticCapability,
    rumbleSteps,
} from './gamepad-haptics.js';

const EFFECT_SETTLE_GRACE_MS = 250;

export function useGamepadRumble(gamepadIndex) {
    const generationRef = useRef(0);
    const mountedRef = useRef(true);
    const [activePattern, setActivePattern] = useState(null);

    const stop = useCallback(async () => {
        generationRef.current += 1;
        if (mountedRef.current) setActivePattern(null);
        await resetCurrentActuator(gamepadIndex);
    }, [gamepadIndex]);

    const play = useCallback(async (pattern, low, high) => {
        const generation = generationRef.current + 1;
        generationRef.current = generation;
        setActivePattern(pattern);
        const capability = currentCapability(gamepadIndex);
        if (!capability.mode) {
            setActivePattern(null);
            return 'This browser does not expose a controllable haptic actuator.';
        }
        const steps = rumbleSteps(pattern, low, high, capability.mode);
        try {
            for (const step of steps) {
                if (generationRef.current !== generation) return null;
                await playStep(capability, step);
            }
        } catch (error) {
            return error instanceof Error ? error.message : String(error);
        } finally {
            if (mountedRef.current && generationRef.current === generation) {
                setActivePattern(null);
            }
        }
        return null;
    }, [gamepadIndex]);

    useEffect(() => {
        mountedRef.current = true;
        const halt = () => {
            generationRef.current += 1;
            if (mountedRef.current) setActivePattern(null);
            resetCurrentActuator(gamepadIndex).catch(() => {});
        };
        const onVisibilityChange = () => {
            if (document.visibilityState === 'hidden') halt();
        };
        const onDisconnected = event => {
            if (event.gamepad?.index === gamepadIndex) halt();
        };
        document.addEventListener('visibilitychange', onVisibilityChange);
        window.addEventListener('gamepaddisconnected', onDisconnected);
        return () => {
            mountedRef.current = false;
            generationRef.current += 1;
            resetCurrentActuator(gamepadIndex).catch(() => {});
            document.removeEventListener('visibilitychange', onVisibilityChange);
            window.removeEventListener('gamepaddisconnected', onDisconnected);
        };
    }, [gamepadIndex]);

    return { activePattern, play, stop };
}

function currentCapability(gamepadIndex) {
    if (typeof navigator === 'undefined' || typeof navigator.getGamepads !== 'function') {
        return hapticCapability(null);
    }
    const gamepad = Array.from(navigator.getGamepads() || [])
        .find(candidate => candidate?.index === gamepadIndex);
    return hapticCapability(gamepad);
}

async function playStep(capability, step) {
    if (capability.mode === HAPTIC_MODE_DUAL_RUMBLE) {
        const effect = capability.actuator.playEffect(HAPTIC_MODE_DUAL_RUMBLE, {
            startDelay: 0,
            duration: step.duration,
            strongMagnitude: step.strongMagnitude,
            weakMagnitude: step.weakMagnitude,
        });
        await settle(effect, step.duration);
        return;
    }
    if (capability.mode === HAPTIC_MODE_PULSE) {
        await settle(capability.actuator.pulse(step.intensity, step.duration), step.duration);
    }
}

async function resetCurrentActuator(gamepadIndex) {
    const capability = currentCapability(gamepadIndex);
    const actuator = capability.actuator;
    if (!actuator) return;
    if (typeof actuator.reset === 'function') {
        await settleReset(actuator.reset());
        return;
    }
    if (capability.mode === HAPTIC_MODE_DUAL_RUMBLE) {
        await settleReset(actuator.playEffect(HAPTIC_MODE_DUAL_RUMBLE, {
            duration: 0,
            strongMagnitude: 0,
            weakMagnitude: 0,
        }));
        return;
    }
    if (capability.mode === HAPTIC_MODE_PULSE) await settleReset(actuator.pulse(0, 0));
}

async function settle(effect, duration) {
    await Promise.all([
        delay(duration),
        Promise.race([
            Promise.resolve(effect),
            delay(duration + EFFECT_SETTLE_GRACE_MS),
        ]),
    ]);
}

async function settleReset(effect) {
    await Promise.race([
        Promise.resolve(effect),
        delay(EFFECT_SETTLE_GRACE_MS),
    ]);
}

function delay(duration) {
    return new Promise(resolve => window.setTimeout(resolve, duration));
}
