import { html } from '../lib/html.js';
import { useMemo } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../components/app/view-keyboard-context.js';

import { PageHeader } from '../components/PageHeader.js';
import { ShortcutEditModal } from './shortcuts/modal.js';
import { useShortcuts } from './shortcuts/use-shortcuts.js';
import { ShortcutsList } from './shortcuts/list.js';

export function ShortcutsView() {
    const { searchQuery } = usePaletteContext();
    const sc = useShortcuts(searchQuery);
    useRegisterViewKeyboard('shortcuts', sc.handleKey, sc.isBlocking);

    const selected = sc.filtered[sc.selectedIndex];
    const commands = useMemo(() => [
        { id: 'shortcuts:add', label: 'Add new shortcut', run: () => sc.openEditModal() },
        { id: 'shortcuts:delete', label: 'Delete selected shortcut', run: () => { if (selected) sc.deleteById(selected.id); } },
        { id: 'shortcuts:edit', label: 'Edit selected shortcut', run: () => { if (selected) sc.openEditModal(selected); } },
        { id: 'shortcuts:run', label: 'Run selected shortcut', run: () => { if (selected) sc.runById(selected.id); } },
    ], [selected, sc.openEditModal, sc.deleteById, sc.runById]);
    useRegisterCommands('shortcuts', commands);

    return html`
        <div class="view-container">
            <${PageHeader} title="Shortcuts" subtitle="User-defined launcher shortcuts for URLs and apps" />
            <div class="view-body">
                <${ShortcutsList} shortcuts=${sc.filtered}
                    selectedIndex=${sc.selectedIndex} onSelect=${sc.setSelectedId} onEdit=${sc.openEditModal} />
            </div>
            ${sc.editModal && html`<${ShortcutEditModal} modal=${sc.editModal} fieldProps=${sc.fieldProps}
                onChange=${sc.handleModalChange} onClose=${sc.closeModal} onSave=${sc.saveShortcut} />`}
        </div>
    `;
}
