import { html } from '../html.js';
import { useCallback, useEffect, useRef } from 'preact/hooks';
import { useSharedSlot } from '../hooks/useSharedSlot.js';
import { useRegisterViewKeyboard } from '../../app/view-keyboard-context.js';
import { setActiveModalContainer } from '../hooks/useModalKeyboard.js';
import { PageHeader } from '../../components/PageHeader.js';
import { SurfaceContainer } from './SurfaceContainer.js';

/**
 * Shared shell for editor sub-pages reached by dive (hotkeys, shortcuts,
 * task-runner). Subscribes to a createSharedSlot, registers a view-keyboard
 * binding for the sub-page id, and wraps the form body in the canonical
 * five-deep page chrome.
 *
 * Props:
 *  - slot: createSharedSlot whose value carries `{ modal, handleKey, isBlocking, ... }`
 *  - viewId: keyboard registration id (e.g. 'hotkeys-editor')
 *  - fallbackTitle, fallbackSubtitle: shown when slot.modal is null
 *  - renderHeader(value): returns the PageHeader for the active editor
 *  - children(value): returns the form body (typically wrapping `.edit-modal-content`)
 */
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
    useEffect(() => {
        if (!hasModal) return;
        setActiveModalContainer(containerRef);
        return () => setActiveModalContainer(null);
    }, [hasModal]);

    if (!value?.modal) {
        return html`<div class="view-container content-shell">
            <${PageHeader} title=${fallbackTitle} subtitle=${fallbackSubtitle} />
        </div>`;
    }

    return html`
        <div class="view-container content-shell">
            ${renderHeader(value)}
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame" containerRef=${containerRef}>
                        ${children(value)}
                    <//>
                </div>
            </div>
        </div>
    `;
}
