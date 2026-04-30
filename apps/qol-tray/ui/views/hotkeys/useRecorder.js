import { useCallback, useRef, useState } from 'preact/hooks';
import { applyRecordingKey } from './recorder.js';

export function useRecorder({ onCapture, onCancel } = {}) {
    const [state, setState] = useState({ isRecording: false, key: '' });
    const stateRef = useRef(state);
    stateRef.current = state;

    const start = useCallback((seed = '') => {
        setState({ isRecording: true, key: seed });
        onCapture?.(seed);
    }, [onCapture]);

    const cancel = useCallback(() => {
        if (!stateRef.current.isRecording) return;
        setState((prev) => ({ ...prev, isRecording: false }));
        onCancel?.();
    }, [onCancel]);

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
        setState({ isRecording: nextRecording, key: nextKey });
        if (nextKey !== prev.key) onCapture?.(nextKey);
        if (!nextRecording && !result.advance) onCancel?.();
        return true;
    }, [onCapture, onCancel]);

    return {
        isRecording: state.isRecording,
        key: state.key,
        start,
        cancel,
        handleKey,
    };
}
