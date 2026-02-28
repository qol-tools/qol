import { useState, useCallback } from 'preact/hooks';

export function useFeedback() {
    const [feedback, setFeedback] = useState(null);
    const set = useCallback((type, message) => setFeedback({ type, message }), []);
    const clear = useCallback(() => setFeedback(null), []);
    return { feedback, setFeedback: set, clearFeedback: clear };
}
