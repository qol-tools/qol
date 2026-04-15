import { html } from '../lib/html.js';
import { useRef, useMemo, useState, useEffect } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../app/view-keyboard-context.js';
import { matchesQuery } from '../utils/collections.js';

import { PageHeader } from '../components/PageHeader.js';
import { SurfaceContainer } from '../lib/components/SurfaceContainer.js';
import { PluginSelect, ActionSelect, KeyInput } from './hotkeys/modal.js';
import { useHotkeys } from './hotkeys/use-hotkeys.js';
import { HotkeysList } from './hotkeys/list.js';

import { createSharedSlot } from '../lib/shared-slot.js';
const editSlot = createSharedSlot({ modal: null, plugins: [], fieldProps: () => ({}), handlers: {} });

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
    useEffect(() => {
        editSlot.set({
            modal: hk.editModal,
            plugins: hk.plugins,
            fieldProps: hk.fieldProps,
            handlers: {
                onPluginChange: hk.handlePluginChange,
                onActionChange: hk.handleActionChange,
                onStartRecording: hk.startRecording,
                onClose: hk.closeModal,
                onSave: hk.saveHotkey,
            },
        });
    }, [hk.editModal, hk.plugins]);
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
        <div class="view-container content-shell">
            <${PageHeader} title="Hotkeys" subtitle="Configure global keyboard shortcuts for plugin actions" />
            ${hk.registrationErrors.length > 0 && html`<${RegistrationWarnings} errors=${hk.registrationErrors} />`}
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame">
                        <${HotkeysList} hotkeys=${filtered} plugins=${hk.plugins}
                            selectedIndex=${hk.selectedIndex} onSelect=${hk.setSelectedIndex} onEdit=${hk.openEditModal} />
                    <//>
                </div>
            </div>
        </div>
    `;
}

export function HotkeyEditorSubPage() {
    const [, bump] = useState(0);
    useEffect(() => editSlot.subscribe(() => bump(t => t + 1)), []);

    const { modal, plugins, fieldProps, handlers } = editSlot.get();
    if (!modal) {
        return html`<div class="view-container content-shell">
            <${PageHeader} title="Hotkey Editor" subtitle="Select a hotkey to edit" />
        </div>`;
    }
    return html`
        <div class="view-container content-shell">
            <${PageHeader} title="Edit Hotkey" subtitle=${`Editing: ${modal.key || 'new hotkey'}`} />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame">
                        <div class="edit-modal-content">
                            <div class="form-group" ...${fieldProps(0)}>
                                <label>Plugin</label>
                                <${PluginSelect} modal=${modal} plugins=${plugins} onChange=${handlers.onPluginChange} />
                            </div>
                            <div class="form-group" ...${fieldProps(1)}>
                                <label>Action</label>
                                <${ActionSelect} modal=${modal} onChange=${handlers.onActionChange} />
                            </div>
                            <div class="form-group" ...${fieldProps(2)}>
                                <label>Shortcut</label>
                                <${KeyInput} modal=${modal} onStartRecording=${handlers.onStartRecording} />
                            </div>
                        </div>
                    <//>
                </div>
            </div>
        </div>
    `;
}
