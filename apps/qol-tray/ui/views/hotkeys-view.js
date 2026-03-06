import { html } from '../lib/html.js';
import { useFooterShortcuts } from '../hooks/useFooterShortcuts.js';
import { PageHeader } from '../components/PageHeader.js';
import { HotkeyEditModal } from './hotkeys/modal.js';
import { useHotkeys } from './hotkeys/use-hotkeys.js';
import { HotkeysList } from './hotkeys/list.js';

const SHORTCUTS = [
    { key: '↑↓', label: 'navigate' },
    { key: 'Enter', label: 'edit' },
    { key: 'a', label: 'add' },
    { key: 'd', label: 'delete' }
];

export function HotkeysView() {
    const hk = useHotkeys();
    useFooterShortcuts(SHORTCUTS);
    HotkeysView.handleKey = hk.handleKey;
    HotkeysView.isBlocking = hk.isBlocking;
    return html`
        <div class="view-container">
            <${PageHeader} title="Hotkeys" subtitle="Configure global keyboard shortcuts for plugin actions" />
            <div class="view-body">
                <${HotkeysList} hotkeys=${hk.hotkeys} plugins=${hk.plugins}
                    selectedIndex=${hk.selectedIndex} onSelect=${hk.setSelectedIndex} onEdit=${hk.openEditModal} />
            </div>
            ${hk.editModal && html`<${HotkeyEditModal} modal=${hk.editModal} plugins=${hk.plugins}
                onPluginChange=${hk.handlePluginChange} onActionChange=${hk.handleActionChange}
                onStartRecording=${hk.startRecording} onClose=${hk.closeModal} onSave=${hk.saveHotkey} />`}
        </div>
    `;
}
