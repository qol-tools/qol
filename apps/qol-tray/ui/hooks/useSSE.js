import { useEffect, useRef } from 'preact/hooks';
import { subscribe, onReconnect } from '../events.js';

export function useSSE(handler) {
    const handlerRef = useRef(handler);
    handlerRef.current = handler;

    useEffect(() => {
        return subscribe((event) => handlerRef.current(event));
    }, []);
}

export function useSSEReconnect(handler) {
    const handlerRef = useRef(handler);
    handlerRef.current = handler;

    useEffect(() => {
        return onReconnect(() => handlerRef.current());
    }, []);
}
