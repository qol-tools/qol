import { html } from '../lib/html.js';
import { useEffect, useMemo } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../app/view-keyboard-context.js';
import { takePendingShortcutPrefill, subscribeShortcutPrefill } from '../lib/deeplink-intent.js';
import { diveViaSelector } from '../lib/world-navigation-singleton.js';

import { PageHeader } from '../components/PageHeader.js';
import { PageShell } from '../components/PageShell.js';
import { DiveEditorSubPage } from '../lib/components/DiveEditorSubPage.js';
import { useDiveEditor } from '../lib/hooks/useDiveEditor.js';
import { ShortcutEditForm } from './shortcuts/modal.js';
import { useShortcuts } from './shortcuts/use-shortcuts.js';
import { ShortcutsList } from './shortcuts/list.js';
import { KeyLegend } from '../lib/components/KeyLegend.js';
import { useViewBindings } from '../lib/hooks/useViewBindings.js';

import { createSharedSlot } from '../lib/shared-slot.js';
export const shortcutEditorSlot = createSharedSlot({
    modal: null,
    fieldProps: () => ({}),
    handlers: {},
    handleKey: null,
    isBlocking: null,
});

export function ShortcutsView() {
    const { searchQuery } = usePaletteContext();
    const sc = useShortcuts(searchQuery);
    useEffect(() => {
        const consume = () => {
            const pending = takePendingShortcutPrefill();
            if (!pending) return;
            sc.openEditModal(pending.shortcut, { editing: pending.editing });
            // The dive target may not be registered yet (parent registers after this
            // child mounts) and the boot camera may still be animating, so retry across
            // frames until the dive into the editor actually takes.
            let attempts = 12;
            const tryDive = () => {
                if (diveViaSelector('[data-view-id="shortcuts"]')) return;
                if (attempts-- > 0) requestAnimationFrame(tryDive);
            };
            requestAnimationFrame(tryDive);
        };
        consume();                              // prefill stashed before this view mounted
        return subscribeShortcutPrefill(consume); // prefill arriving after mount (boot order / warm nav)
    }, []);
    useRegisterViewKeyboard('shortcuts', sc.handleKey, sc.isBlocking);

    useDiveEditor({
        slot: shortcutEditorSlot,
        deps: [sc.editModal, sc.handleKey, sc.isBlocking],
        build: () => ({
            modal: sc.editModal,
            fieldProps: sc.fieldProps,
            handlers: {
                onChange: sc.handleModalChange,
                onClose: sc.closeModal,
                onSave: sc.saveShortcut,
            },
            handleKey: sc.handleKey,
            isBlocking: sc.isBlocking,
        }),
    });

    const selected = sc.filtered[sc.selectedIndex];
    const commands = useMemo(() => [
        { id: 'shortcuts:add', label: 'Add new shortcut', run: () => sc.openEditModal() },
        { id: 'shortcuts:delete', label: 'Delete selected shortcut', run: () => { if (selected) sc.deleteById(selected.id); } },
        { id: 'shortcuts:edit', label: 'Edit selected shortcut', run: () => { if (selected) sc.openEditModal(selected); } },
        { id: 'shortcuts:run', label: 'Run selected shortcut', run: () => { if (selected) sc.runById(selected.id); } },
    ], [selected, sc.openEditModal, sc.deleteById, sc.runById]);
    useRegisterCommands('shortcuts', commands);

    const bindings = useViewBindings('shortcuts');
    return html`
        <${PageShell}
            subtitle="User-defined launcher shortcuts for URLs and apps"
            aside=${html`<${KeyLegend} bindings=${bindings} />`}>
            <${ShortcutsList} shortcuts=${sc.filtered}
                selectedIndex=${sc.selectedIndex} onSelect=${sc.setSelectedId} onEdit=${sc.openEditModal} />
        <//>
    `;
}

export function ShortcutEditorSubPage({ slot, viewId = 'shortcuts-editor' }) {
    return html`<${DiveEditorSubPage}
        slot=${slot}
        viewId=${viewId}
        fallbackTitle="Shortcut Editor"
        fallbackSubtitle="Select a shortcut to edit"
        renderHeader=${(v) => html`<${PageHeader}
            title=${v.modal.editing ? 'Edit Shortcut' : 'Add Shortcut'}
            subtitle=${v.modal.shortcut.name || v.modal.shortcut.id || 'new shortcut'} />`}
        children=${(v) => html`<${ShortcutEditForm}
            modal=${v.modal} fieldProps=${v.fieldProps}
            onChange=${v.handlers.onChange} onClose=${v.handlers.onClose} onSave=${v.handlers.onSave} />`} />`;
}
