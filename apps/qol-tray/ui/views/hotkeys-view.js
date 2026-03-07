import { html } from '../lib/html.js';
import { useRef, useMemo } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';

import { PageHeader } from '../components/PageHeader.js';
import { HotkeyEditModal } from './hotkeys/modal.js';
import { useHotkeys } from './hotkeys/use-hotkeys.js';
import { HotkeysList } from './hotkeys/list.js';


function filterHotkeys(hotkeys, plugins, query) {
    if (!query) return hotkeys;
    const q = query.toLowerCase();
    return hotkeys.filter(hk => {
        const plugin = plugins.find(p => p.id === hk.plugin_id);
        return hk.key.toLowerCase().includes(q)
            || (plugin?.name || hk.plugin_id).toLowerCase().includes(q)
            || hk.action.toLowerCase().includes(q);
    });
}

export function HotkeysView() {
    const hk = useHotkeys();
    const { searchQuery } = usePaletteContext();
    const filtered = useMemo(
        () => filterHotkeys(hk.hotkeys, hk.plugins, searchQuery),
        [hk.hotkeys, hk.plugins, searchQuery]
    );

    HotkeysView.handleKey = hk.handleKey;
    HotkeysView.isBlocking = hk.isBlocking;

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
            <div class="view-body">
                <${HotkeysList} hotkeys=${filtered} plugins=${hk.plugins}
                    selectedIndex=${hk.selectedIndex} onSelect=${hk.setSelectedIndex} onEdit=${hk.openEditModal} />
            </div>
            ${hk.editModal && html`<${HotkeyEditModal} modal=${hk.editModal} plugins=${hk.plugins}
                onPluginChange=${hk.handlePluginChange} onActionChange=${hk.handleActionChange}
                onStartRecording=${hk.startRecording} onClose=${hk.closeModal} onSave=${hk.saveHotkey} />`}
        </div>
    `;
}
