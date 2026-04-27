import { html } from '../lib/html.js';
import { useMemo, useState, useEffect, useCallback } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../app/view-keyboard-context.js';

import { PageHeader } from '../components/PageHeader.js';
import { SurfaceContainer } from '../lib/components/SurfaceContainer.js';
import { ShortcutEditForm } from './shortcuts/modal.js';
import { useShortcuts } from './shortcuts/use-shortcuts.js';
import { ShortcutsList } from './shortcuts/list.js';

import { createSharedSlot } from '../lib/shared-slot.js';
const editSlot = createSharedSlot({
    modal: null,
    fieldProps: () => ({}),
    handlers: {},
    handleKey: null,
    isBlocking: null,
});

export function ShortcutsView() {
    const { searchQuery } = usePaletteContext();
    const sc = useShortcuts(searchQuery);
    useRegisterViewKeyboard('shortcuts', sc.handleKey, sc.isBlocking);

    useEffect(() => {
        editSlot.set({
            modal: sc.editModal,
            fieldProps: sc.fieldProps,
            handlers: {
                onChange: sc.handleModalChange,
                onClose: sc.closeModal,
                onSave: sc.saveShortcut,
            },
            handleKey: sc.handleKey,
            isBlocking: sc.isBlocking,
        });
    }, [sc.editModal, sc.handleKey, sc.isBlocking]);

    const selected = sc.filtered[sc.selectedIndex];
    const commands = useMemo(() => [
        { id: 'shortcuts:add', label: 'Add new shortcut', run: () => sc.openEditModal() },
        { id: 'shortcuts:delete', label: 'Delete selected shortcut', run: () => { if (selected) sc.deleteById(selected.id); } },
        { id: 'shortcuts:edit', label: 'Edit selected shortcut', run: () => { if (selected) sc.openEditModal(selected); } },
        { id: 'shortcuts:run', label: 'Run selected shortcut', run: () => { if (selected) sc.runById(selected.id); } },
    ], [selected, sc.openEditModal, sc.deleteById, sc.runById]);
    useRegisterCommands('shortcuts', commands);

    return html`
        <div class="view-container content-shell">
            <${PageHeader} subtitle="User-defined launcher shortcuts for URLs and apps" />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame">
                        <${ShortcutsList} shortcuts=${sc.filtered}
                            selectedIndex=${sc.selectedIndex} onSelect=${sc.setSelectedId} onEdit=${sc.openEditModal} />
                    <//>
                </div>
            </div>
        </div>
    `;
}

export function ShortcutEditorSubPage() {
    const [, bump] = useState(0);
    useEffect(() => editSlot.subscribe(() => bump(t => t + 1)), []);

    const slotHandleKey = useCallback((e) => {
        const fn = editSlot.get().handleKey;
        if (fn) fn(e);
    }, []);
    const slotIsBlocking = useCallback(() => {
        const fn = editSlot.get().isBlocking;
        return fn ? fn() : false;
    }, []);
    useRegisterViewKeyboard('shortcuts-editor', slotHandleKey, slotIsBlocking);

    const { modal, fieldProps, handlers } = editSlot.get();
    if (!modal) {
        return html`<div class="view-container content-shell">
            <${PageHeader} title="Shortcut Editor" subtitle="Select a shortcut to edit" />
        </div>`;
    }
    return html`
        <div class="view-container content-shell">
            <${PageHeader} title=${modal.editing ? 'Edit Shortcut' : 'Add Shortcut'}
                subtitle=${modal.shortcut.name || modal.shortcut.id || 'new shortcut'} />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame">
                        <${ShortcutEditForm} modal=${modal} fieldProps=${fieldProps}
                            onChange=${handlers.onChange} onClose=${handlers.onClose} onSave=${handlers.onSave} />
                    <//>
                </div>
            </div>
        </div>
    `;
}
