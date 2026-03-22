import { html } from '../lib/html.js';
import { useRef, useMemo } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../components/app/view-keyboard-context.js';
import { matchesQuery } from '../utils/collections.js';

import { PageHeader } from '../components/PageHeader.js';
import { HotkeyEditModal } from './hotkeys/modal.js';
import { useHotkeys } from './hotkeys/use-hotkeys.js';
import { HotkeysList } from './hotkeys/list.js';

function RegistrationWarnings({ errors }) {
    return html`
        <div class="hotkeys-warnings">
            ${errors.map(err => html`
                <div key=${err.key} class="hotkeys-warning-item">
                    <kbd>${err.key}</kbd>
                    <span>${err.error}</span>
                </div>
            `)}
        </div>
    `;
}

export function HotkeysView() {
    const hk = useHotkeys();
    const { searchQuery } = usePaletteContext();
    const filtered = useMemo(
        () => searchQuery
            ? hk.hotkeys.filter(h => {
                const plugin = hk.plugins.find(p => p.id === h.plugin_id);
                return matchesQuery([h.key, plugin?.name || h.plugin_id, h.action], searchQuery);
            })
            : hk.hotkeys,
        [hk.hotkeys, hk.plugins, searchQuery]
    );
    useRegisterViewKeyboard('hotkeys', hk.handleKey, hk.isBlocking);

    const hkRef = useRef(hk);
    hkRef.current = hk;
    const filteredRef = useRef(filtered);
    filteredRef.current = filtered;
    const commands = useMemo(() => [
        { id: 'hotkeys:add', label: 'Add new hotkey', run: () => hkRef.current.openEditModal() },
        { id: 'hotkeys:delete', label: 'Delete selected hotkey', run: () => hkRef.current.deleteSelected() },
        { id: 'hotkeys:edit', label: 'Edit selected hotkey', run: () => { const h = filteredRef.current[hkRef.current.selectedIndex]; if (h) hkRef.current.openEditModal(h); } },
    ], []);
    useRegisterCommands('hotkeys', commands);

    return html`
        <div class="view-container">
            <${PageHeader} title="Hotkeys" subtitle="Configure global keyboard shortcuts for plugin actions" />
            ${hk.registrationErrors.length > 0 && html`<${RegistrationWarnings} errors=${hk.registrationErrors} />`}
            <div class="view-body">
                <${HotkeysList} hotkeys=${filtered} plugins=${hk.plugins}
                    selectedIndex=${hk.selectedIndex} onSelect=${hk.setSelectedIndex} onEdit=${hk.openEditModal} />
            </div>
            ${hk.editModal && html`<${HotkeyEditModal} modal=${hk.editModal} plugins=${hk.plugins}
                fieldProps=${hk.fieldProps} onPluginChange=${hk.handlePluginChange} onActionChange=${hk.handleActionChange}
                onStartRecording=${hk.startRecording} onClose=${hk.closeModal} onSave=${hk.saveHotkey} />`}
        </div>
    `;
}
