import { html } from '../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { createDebug } from '../lib/debug.js';
import { AUTH_LOST_EVENT, isAuthLost } from '../lib/http-auth.js';
import { streamStatus } from '../events.js';
import { Button } from '../lib/components/Button.js';

const log = createDebug('qol:auth');

const FOCUSABLE = 'button:not([disabled]), [href], [tabindex]:not([tabindex="-1"])';

export function AuthRecoveryOverlay() {
    const [lost, setLost] = useState(isAuthLost());
    const panelRef = useRef(null);

    useEffect(() => {
        const onLost = () => {
            log('auth-lost event → recovery overlay shown');
            setLost(true);
        };
        window.addEventListener(AUTH_LOST_EVENT, onLost);
        return () => window.removeEventListener(AUTH_LOST_EVENT, onLost);
    }, []);

    useEffect(() => {
        if (!lost) return;
        const first = panelRef.current?.querySelector('button');
        if (first && !first.matches(':focus')) first.focus();
    }, [lost]);

    const handleKeyDown = useCallback((event) => {
        if (event.key === 'Tab') {
            event.stopPropagation();
            const focusables = Array.from(
                panelRef.current?.querySelectorAll(FOCUSABLE) || []
            ).filter((el) => el.offsetParent !== null);
            if (focusables.length === 0) return;
            const first = focusables[0];
            const last = focusables[focusables.length - 1];
            const active = document.activeElement;
            if (event.shiftKey && active === first) {
                event.preventDefault();
                last.focus();
            } else if (!event.shiftKey && active === last) {
                event.preventDefault();
                first.focus();
            } else if (!panelRef.current?.contains(active)) {
                event.preventDefault();
                first.focus();
            }
            return;
        }
        if (event.key === 'Escape') {
            event.stopPropagation();
            setLost(false);
        }
    }, []);

    const dismiss = useCallback(() => setLost(false), []);

    if (!lost) return null;

    const statusLabel = streamStatus() === 'stopped' ? 'Live updates stopped' : 'Live updates paused';

    return html`
        <div class="auth-recovery-overlay" ref=${panelRef} onKeyDown=${handleKeyDown}
            role="dialog" aria-modal="true" aria-labelledby="auth-recovery-title">
            <div class="auth-recovery-panel">
                <h3 id="auth-recovery-title">Dashboard connection expired</h3>
                <p>This tab lost access to the local QoL server. Open the dashboard from the QoL tray menu to get a fresh connection, then close this tab.</p>
                <div class="modal-footer-actions">
                    <${Button} variant="btn-ghost" onActivate=${dismiss}>Close</${Button}>
                </div>
                <p class="auth-recovery-status">${statusLabel}</p>
            </div>
        </div>
    `;
}
