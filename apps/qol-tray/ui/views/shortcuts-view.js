import { html } from '../lib/html.js';
import { useMemo, useState, useEffect } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../components/app/view-keyboard-context.js';

import { PageHeader } from '../components/PageHeader.js';
import { SurfaceContainer } from '../components/SurfaceContainer.js';
import { ShortcutEditForm } from './shortcuts/modal.js';
import { useShortcuts } from './shortcuts/use-shortcuts.js';
import { ShortcutsList } from './shortcuts/list.js';

// Shared state: ShortcutsView writes, ShortcutEditorSubPage reads
const _sharedEdit = { modal: null, fieldProps: () => ({}), handlers: {} };
const _editListeners = new Set();
function notifyEditChange() { for (const fn of _editListeners) fn(); }
function subscribeEditState(fn) { _editListeners.add(fn); return () => _editListeners.delete(fn); }

export function ShortcutsView() {
    const { searchQuery } = usePaletteContext();
    const sc = useShortcuts(searchQuery);
    useRegisterViewKeyboard('shortcuts', sc.handleKey, sc.isBlocking);

    useEffect(() => {
        _sharedEdit.modal = sc.editModal;
        _sharedEdit.fieldProps = sc.fieldProps;
        _sharedEdit.handlers = {
            onChange: sc.handleModalChange,
            onClose: sc.closeModal,
            onSave: sc.saveShortcut,
        };
        notifyEditChange();
    }, [sc.editModal]);

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
            <${PageHeader} title="Shortcuts" subtitle="User-defined launcher shortcuts for URLs and apps" />
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
    useEffect(() => subscribeEditState(() => bump(t => t + 1)), []);

    const { modal, fieldProps, handlers } = _sharedEdit;
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
