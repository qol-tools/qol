import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { applyRecordingKey, isNativeRecordingCancel, recordedNativeKey } from './recorder.js';
import { createKeyguardPause } from '../../lib/keyguard.js';
import { useSSE } from '../../hooks/useSSE.js';

let nextSessionId = Date.now() * 1000;

export function useRecorder({ onCapture, onCancel } = {}) {
    const [state, setState] = useState({ isRecording: false, key: '' });
    const stateRef = useRef(state);
    stateRef.current = state;
    const pauserRef = useRef(null);
    const sessionRef = useRef(0);
    if (!pauserRef.current) pauserRef.current = createKeyguardPause();

    const stopNativeRecording = useCallback(() => {
        const sessionId = sessionRef.current;
        if (!sessionId) return;
        sessionRef.current = 0;
        fetch(`/api/hotkeys/recording/${sessionId}`, {
            method: 'DELETE',
            keepalive: true,
            qolSuppressErrorToast: true,
        }).catch(() => {});
    }, []);

    useEffect(() => () => {
        pauserRef.current.resume();
        stopNativeRecording();
    }, [stopNativeRecording]);

    useSSE((event) => {
        if (!stateRef.current.isRecording) return;
        if (isNativeRecordingCancel(event, sessionRef.current)) {
            sessionRef.current = 0;
            pauserRef.current.resume();
            setState((prev) => ({ ...prev, isRecording: false }));
            onCancel?.();
            return;
        }
        const key = recordedNativeKey(event, sessionRef.current);
        if (!key) return;
        sessionRef.current = 0;
        pauserRef.current.resume();
        setState({ isRecording: false, key });
        onCapture?.(key);
    });

    const start = useCallback((seed = '') => {
        stopNativeRecording();
        const sessionId = ++nextSessionId;
        sessionRef.current = sessionId;
        pauserRef.current.pause();
        setState({ isRecording: true, key: seed });
        onCapture?.(seed);
        fetch(`/api/hotkeys/recording/${sessionId}`, {
            method: 'POST',
            qolSuppressErrorToast: true,
        }).catch(() => {});
    }, [onCapture, stopNativeRecording]);

    const cancel = useCallback(() => {
        if (!stateRef.current.isRecording) return;
        stopNativeRecording();
        pauserRef.current.resume();
        setState((prev) => ({ ...prev, isRecording: false }));
        onCancel?.();
    }, [onCancel, stopNativeRecording]);

    const handleKey = useCallback((event) => {
        if (!stateRef.current.isRecording) return false;
        event.preventDefault();
        event.stopPropagation();
        const prev = stateRef.current;
        const result = applyRecordingKey(
            { key: prev.key, recording: true },
            event
        );
        const nextKey = result.modal.key;
        const nextRecording = result.modal.recording !== false;
        if (!nextRecording) {
            stopNativeRecording();
            pauserRef.current.resume();
        }
        setState({ isRecording: nextRecording, key: nextKey });
        if (nextKey !== prev.key) onCapture?.(nextKey);
        if (!nextRecording && !result.advance) onCancel?.();
        return true;
    }, [onCapture, onCancel, stopNativeRecording]);

    return {
        isRecording: state.isRecording,
        key: state.key,
        start,
        cancel,
        handleKey,
    };
}
