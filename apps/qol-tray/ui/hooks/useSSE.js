import { useEffect } from 'preact/hooks';
import { subscribe, onReconnect } from '../events.js';

export function useSSE(handler) {
    useEffect(() => subscribe(handler), [handler]);
}

export function useSSEReconnect(handler) {
    useEffect(() => onReconnect(handler), [handler]);
}
