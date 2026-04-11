import { html } from '../../../lib/html.js';
import { useCallback, useState } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { useDispatchAction } from '../../../hooks/useDispatchAction.js';
import { fieldSelectionClasses } from '../field-map.js';
import { Modal, ModalActions } from '../../../components/ModalPreact.js';

export function ActionField({ field }) {
    const ctx = usePluginConfigContext();
    const selected = ctx.selectedFieldId === field.id;
    const index = ctx.fieldIndexById[field.id];
    const runtimeAction = ctx.runtime?.action?.[field.action];
    const { dispatch, pending, error } = useDispatchAction(ctx.pluginId, field.action);
    const [confirmOpen, setConfirmOpen] = useState(false);

    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    const runWithoutConfirm = useCallback(() => {
        dispatch().catch(() => {});
    }, [dispatch]);

    const onActivate = useCallback(() => {
        if (runtimeAction?.confirm) {
            setConfirmOpen(true);
            return;
        }
        runWithoutConfirm();
    }, [runtimeAction, runWithoutConfirm]);

    const onConfirmed = useCallback(() => {
        setConfirmOpen(false);
        runWithoutConfirm();
    }, [runWithoutConfirm]);

    const onCancel = useCallback(() => {
        setConfirmOpen(false);
    }, []);

    const onKeyDown = useCallback((event) => {
        if (event.key !== 'Enter' && event.key !== ' ') {
            return;
        }
        event.preventDefault();
        event.stopPropagation();
        onActivate();
    }, [onActivate]);

    const variant = field.variant || 'primary';

    return html`
        <div class="field-group field-action ${fieldSelectionClasses(selected)}"
            data-plugin-config-field-id=${field.id}
            data-plugin-config-index=${index}
            data-selected-surface="" tabIndex="-1"
            data-selected=${selected ? 'true' : 'false'}
            onMouseDown=${onSelect}
            onFocus=${onSelect}
            onKeyDown=${onKeyDown}>
            <button type="button" class="btn btn-${variant}"
                    disabled=${pending}
                    onClick=${onActivate}>
                ${pending ? 'Working...' : (field.label || 'Run')}
            </button>
            ${error && html`<div class="field-action-error">${error}</div>`}
            ${confirmOpen && html`
                <${Modal} open=${true} onClose=${onCancel} className="edit-modal">
                    <div class="edit-modal-content">
                        <p>${runtimeAction?.confirm || 'Confirm action?'}</p>
                        <${ModalActions} onClose=${onCancel} onSave=${onConfirmed} />
                    </div>
                <//>
            `}
        </div>
    `;
}
