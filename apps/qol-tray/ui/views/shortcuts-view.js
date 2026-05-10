import { html } from '../lib/html.js';
import { useMemo } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../app/view-keyboard-context.js';

import { PageHeader } from '../components/PageHeader.js';
import { SurfaceContainer } from '../lib/components/SurfaceContainer.js';
import { DiveEditorSubPage } from '../lib/components/DiveEditorSubPage.js';
import { useDiveEditor } from '../lib/hooks/useDiveEditor.js';
import { ShortcutEditForm } from './shortcuts/modal.js';
import { useShortcuts } from './shortcuts/use-shortcuts.js';
import { ShortcutsList } from './shortcuts/list.js';

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
