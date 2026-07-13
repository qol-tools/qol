import { useEffect, useRef, useState } from 'preact/hooks';
import {
    appendSignalHistory,
    SIGNAL_SAMPLE_INTERVAL_MS,
    signalHistorySample,
} from './gamepad-model.js';

export function useGamepadSignalHistory(containerRef, snapshot) {
    const [history, setHistory] = useState([]);
    const snapshotRef = useRef(snapshot);
    snapshotRef.current = snapshot;
    const identity = controllerIdentity(snapshot);

    useEffect(() => {
        setHistory([]);
        if (!identity || typeof window === 'undefined' || typeof document === 'undefined') {
            return undefined;
        }

        let timer = null;
        let bluetoothKnown = false;
        let inViewport = typeof IntersectionObserver !== 'function';

        const stop = () => {
            if (timer === null) return;
            clearInterval(timer);
            timer = null;
        };
        const sample = () => {
            const connection = snapshotRef.current?.connection || null;
            const transport = String(connection?.transport || '').toLowerCase();
            if (transport && transport !== 'bluetooth') {
                bluetoothKnown = false;
                setHistory(current => current.length > 0 ? [] : current);
                return;
            }
            const next = signalHistorySample(connection, bluetoothKnown);
            if (transport === 'bluetooth') bluetoothKnown = true;
            if (!next) return;
            setHistory(current => appendSignalHistory(current, next));
        };
        const sync = () => {
            const shouldRun = inViewport && document.visibilityState !== 'hidden';
            if (!shouldRun) {
                stop();
                return;
            }
            if (timer !== null) return;
            sample();
            timer = window.setInterval(sample, SIGNAL_SAMPLE_INTERVAL_MS);
        };
        const observer = createVisibilityObserver(containerRef.current, visible => {
            inViewport = visible;
            sync();
        });
        const onVisibilityChange = () => sync();

        document.addEventListener('visibilitychange', onVisibilityChange);
        sync();
        return () => {
            stop();
            observer?.disconnect();
            document.removeEventListener('visibilitychange', onVisibilityChange);
        };
    }, [containerRef, identity]);

    return history;
}

function controllerIdentity(snapshot) {
    if (!snapshot) return '';
    return `${snapshot.index}:${snapshot.id}`;
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
