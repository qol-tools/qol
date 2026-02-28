import { useEffect, useRef } from 'preact/hooks';
import { subscribe, onReconnect, suspend, resume } from '../events.js';

// Suspend/resume SSE connection based on window focus.
// Tray popups don't trigger document.hidden — only blur/focus works.
let initialized = false;
function initFocusTracking() {
    if (initialized) return;
    initialized = true;
    window.addEventListener('blur', () => suspend());
    window.addEventListener('focus', () => resume());
}

export function useSSE(handler) {
    const handlerRef = useRef(handler);
    handlerRef.current = handler;

    useEffect(() => {
        initFocusTracking();
        return subscribe((event) => handlerRef.current(event));
    }, []);
}

export function useSSEReconnect(handler) {
    const handlerRef = useRef(handler);
    handlerRef.current = handler;

    useEffect(() => {
        initFocusTracking();
        return onReconnect(() => handlerRef.current());
    }, []);
}
