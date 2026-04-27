import { useCallback, useRef, useState } from 'preact/hooks';
import { applyRecordingKey } from './recorder.js';

// Encapsulates the Shortcut field's record-a-chord state machine. Owns
// `isRecording` and the in-progress `key`, delegates every keystroke decision
// to the pure `applyRecordingKey` helper, and notifies the consumer when the
// chord commits or cancels. The modal in `use-hotkeys.js` no longer needs a
// `recording` field — this hook is the single source of truth for that bit.
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
