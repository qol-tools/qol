import { useEffect, useRef, useState } from 'preact/hooks';
import {
    chooseGamepad,
    gamepadSnapshot,
    mergeNativeInputs,
    monitorSignature,
} from './gamepad-model.js';

const INITIAL_STATE = Object.freeze({
    status: 'waiting',
    message: 'Press any controller button to expose it to this page.',
    gamepads: [],
    selected: null,
});

export function useGamepadMonitor(containerRef, preference, nativeQuery = {}) {
    const [state, setState] = useState(INITIAL_STATE);
    const preferenceRef = useRef(preference);
    preferenceRef.current = preference;

    useEffect(() => monitorGamepads(
        containerRef,
        preferenceRef,
        setState,
        nativeQuery,
    ), [nativeQuery.pluginId, nativeQuery.queryName, nativeQuery.intervalMs]);

    return state;
}

function monitorGamepads(containerRef, preferenceRef, setState, nativeQuery) {
    if (typeof navigator === 'undefined' || typeof navigator.getGamepads !== 'function') {
        setState({
            status: 'unsupported',
            message: 'This browser does not expose the Gamepad API.',
            gamepads: [],
            selected: null,
        });
        return undefined;
    }

    let frame = null;
    let visible = false;
    let signature = '';
    let nativeData = null;
    let nativeSignature = '';
    let nativeTimer = null;
    let nativeRequest = null;
    let disposed = false;
    let browserHasGamepad = false;

    const stop = () => {
        if (frame === null) return;
        cancelAnimationFrame(frame);
        frame = null;
    };

    const schedule = () => {
        if (disposed || !visible || document.visibilityState === 'hidden' || frame !== null) return;
        frame = requestAnimationFrame(read);
    };

    const stopNative = () => {
        if (nativeTimer !== null) clearTimeout(nativeTimer);
        nativeTimer = null;
        nativeRequest?.abort();
        nativeRequest = null;
    };

    const scheduleNative = (delay = 0) => {
        if (disposed) return;
        if (!nativeQuery.pluginId || !nativeQuery.queryName) return;
        if (!browserHasGamepad) return;
        if (!visible || document.visibilityState === 'hidden') return;
        if (nativeTimer !== null || nativeRequest !== null) return;
        nativeTimer = setTimeout(readNative, delay);
    };

    const readNative = async () => {
        nativeTimer = null;
        if (disposed || !visible || document.visibilityState === 'hidden') return;
        const request = new AbortController();
        nativeRequest = request;
        try {
            const nextNativeData = await fetchNativeInput(
                nativeQuery.pluginId,
                nativeQuery.queryName,
                request.signal,
            );
            if (disposed) return;
            const nextNativeSignature = JSON.stringify(nextNativeData);
            if (nextNativeSignature !== nativeSignature) {
                nativeData = nextNativeData;
                nativeSignature = nextNativeSignature;
                signature = '';
                schedule();
            }
        } catch (error) {
            if (error?.name !== 'AbortError' && nativeData !== null) {
                nativeData = null;
                nativeSignature = '';
                signature = '';
                schedule();
            }
        } finally {
            if (nativeRequest === request) nativeRequest = null;
            scheduleNative(Math.max(32, Number(nativeQuery.intervalMs) || 50));
        }
    };

    const read = () => {
        frame = null;
        if (!visible || document.visibilityState === 'hidden') return;
        const result = readGamepads();
        if (result.error) {
            setState({
                status: 'blocked',
                message: result.error,
                gamepads: [],
                selected: null,
            });
            return;
        }
        const browserGamepads = result.gamepads.map(gamepadSnapshot);
        const previouslyHadGamepad = browserHasGamepad;
        browserHasGamepad = browserGamepads.some(gamepad => gamepad.connected !== false);
        if (browserHasGamepad && !previouslyHadGamepad) scheduleNative();
        if (!browserHasGamepad && previouslyHadGamepad) {
            stopNative();
            nativeData = null;
            nativeSignature = '';
        }
        const gamepads = mergeNativeInputs(browserGamepads, nativeData);
        const selected = chooseGamepad(gamepads, preferenceRef.current);
        const nextSignature = monitorSignature(gamepads, selected);
        if (nextSignature !== signature) {
            signature = nextSignature;
            setState(selected
                ? { status: 'ready', message: '', gamepads, selected }
                : { ...INITIAL_STATE, gamepads });
        }
        schedule();
    };

    const onVisibilityChange = () => {
        if (document.visibilityState === 'hidden') {
            stop();
            stopNative();
            return;
        }
        schedule();
        scheduleNative();
    };
    const onConnectionChange = () => {
        signature = '';
        schedule();
    };
    const observer = createVisibilityObserver(containerRef.current, isVisible => {
        visible = isVisible;
        if (!visible) {
            stop();
            stopNative();
            return;
        }
        schedule();
        scheduleNative();
    });

    window.addEventListener('gamepadconnected', onConnectionChange);
    window.addEventListener('gamepaddisconnected', onConnectionChange);
    document.addEventListener('visibilitychange', onVisibilityChange);

    return () => {
        disposed = true;
        stop();
        stopNative();
        observer?.disconnect();
        window.removeEventListener('gamepadconnected', onConnectionChange);
        window.removeEventListener('gamepaddisconnected', onConnectionChange);
        document.removeEventListener('visibilitychange', onVisibilityChange);
    };
}

async function fetchNativeInput(pluginId, queryName, signal) {
    const response = await fetch(
        `/api/plugins/${encodeURIComponent(pluginId)}/queries/${encodeURIComponent(queryName)}`,
        { signal },
    );
    if (!response.ok) {
        const message = await response.text();
        throw new Error(message || `HTTP ${response.status}`);
    }
    return response.json();
}

function readGamepads() {
    try {
        return {
            gamepads: Array.from(navigator.getGamepads() || []).filter(Boolean),
            error: null,
        };
    } catch (error) {
        return {
            gamepads: [],
            error: error instanceof Error ? error.message : String(error),
        };
    }
}

function createVisibilityObserver(element, onChange) {
    if (!element || typeof IntersectionObserver !== 'function') {
        onChange(true);
        return null;
    }
    const observer = new IntersectionObserver(entries => {
        onChange(Boolean(entries[0]?.isIntersecting));
    }, { threshold: 0.05 });
    observer.observe(element);
    return observer;
}
