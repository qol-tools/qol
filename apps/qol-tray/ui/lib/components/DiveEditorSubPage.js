import { html } from '../html.js';
import { useCallback, useEffect, useRef } from 'preact/hooks';
import { useSharedSlot } from '../hooks/useSharedSlot.js';
import { useRegisterViewKeyboard } from '../../app/view-keyboard-context.js';
import { setActiveModalContainer } from '../hooks/useModalKeyboard.js';
import { ascend } from '../world-navigation-singleton.js';
import { PageHeader } from '../../components/PageHeader.js';
import { PageShell } from '../../components/PageShell.js';

export function DiveEditorSubPage({
    slot,
    viewId,
    fallbackTitle,
    fallbackSubtitle,
    renderHeader,
    children,
}) {
    const value = useSharedSlot(slot);

    const slotHandleKey = useCallback((e) => {
        const fn = slot.get().handleKey;
        if (fn) fn(e);
    }, [slot]);
    const slotIsBlocking = useCallback(() => {
        const fn = slot.get().isBlocking;
        return fn ? fn() : false;
    }, [slot]);
    useRegisterViewKeyboard(viewId, slotHandleKey, slotIsBlocking);

    const containerRef = useRef(null);
    const hasModal = !!value?.modal;
    const wasModalRef = useRef(hasModal);
    useEffect(() => {
        if (!hasModal) return;
        setActiveModalContainer(containerRef);
        return () => setActiveModalContainer(null);
    }, [hasModal]);
    useEffect(() => {
        if (wasModalRef.current && !hasModal) ascend();
        wasModalRef.current = hasModal;
    }, [hasModal]);

    if (!value?.modal) {
        return html`<${PageShell}
            header=${html`<${PageHeader} title=${fallbackTitle} subtitle=${fallbackSubtitle} />`}
            frame=${false} />`;
    }

    return html`
        <${PageShell} header=${renderHeader(value)} frameRef=${containerRef}>
            ${children(value)}
        <//>
    `;
}
