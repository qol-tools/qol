import { html } from '../lib/html.js';
import { useState, useEffect, useCallback, useRef } from 'preact/hooks';
import { IconCopy } from '../assets/icon-copy.js';
import { IconClose } from '../assets/icon-close.js';

export function GlobalToast() {
    const [toasts, setToasts] = useState([]);
    const timersRef = useRef(new Map());

    const dismiss = useCallback((key) => {
        clearTimeout(timersRef.current.get(key));
        timersRef.current.delete(key);
        setToasts(prev => prev.filter(t => t.key !== key));
    }, []);

    const resetTimer = useCallback((key, ms) => {
        clearTimeout(timersRef.current.get(key));
        timersRef.current.set(key, setTimeout(() => dismiss(key), ms));
    }, [dismiss]);

    useEffect(() => {
        const handler = (e) => {
            const { type, message } = e.detail;
            const key = `${type}:${message}`;
            const ms = type === 'error' ? 6000 : 4000;
            setToasts(prev => {
                const existing = prev.find(t => t.key === key);
                if (existing) {
                    if (!existing.hovered) resetTimer(key, ms);
                    return prev.map(t => t.key !== key ? t : { ...t, count: t.count + 1 });
                }
                resetTimer(key, ms);
                return [...prev, { key, type, message, count: 1, hovered: false }];
            });
        };
        window.addEventListener('app-toast', handler);
        return () => window.removeEventListener('app-toast', handler);
    }, [resetTimer]);

    const pause = useCallback((key) => {
        clearTimeout(timersRef.current.get(key));
        setToasts(prev => prev.map(t => t.key !== key ? t : { ...t, hovered: true }));
    }, []);

    const copy = useCallback((e, message) => {
        e.stopPropagation();
        navigator.clipboard.writeText(message);
    }, []);

    if (toasts.length === 0) return null;
    return html`
        <div class="toast-stack">
            ${toasts.map(t => html`
                <div key=${t.key} class="toast toast-${t.type} ${t.hovered ? 'toast-pinned' : ''}"
                    onMouseEnter=${() => pause(t.key)}>
                    <span class="toast-message">${t.message}</span>
                    ${t.count > 1 && html`<span class="toast-count">${t.count}</span>`}
                    ${t.hovered && html`<span class="toast-actions">
                        <button class="toast-btn" onClick=${(e) => copy(e, t.message)} title="Copy"><${IconCopy} /></button>
                        <button class="toast-btn" onClick=${() => dismiss(t.key)} title="Close"><${IconClose} /></button>
                    </span>`}
                </div>
            `)}
        </div>
    `;
}
