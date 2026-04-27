import { html } from '../lib/html.js';
import { useRef, useMemo, useState, useEffect, useCallback } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../app/view-keyboard-context.js';
import { matchesQuery } from '../utils/collections.js';
import { ascend, diveViaSelector } from '../lib/world-navigation-singleton.js';

import { PageHeader } from '../components/PageHeader.js';
import { SurfaceContainer } from '../lib/components/SurfaceContainer.js';
import { ModalActions } from '../lib/components/ModalPreact.js';
import { PluginSelect, ActionSelect, KeyInput } from './hotkeys/modal.js';
import { useHotkeys } from './hotkeys/use-hotkeys.js';
import { HotkeysList } from './hotkeys/list.js';

import { createSharedSlot } from '../lib/shared-slot.js';
const editSlot = createSharedSlot({
    modal: null,
    plugins: [],
    fieldProps: () => ({}),
    handlers: {},
    handleKey: null,
    isBlocking: null,
});
const HOTKEYS_EDITOR_DIVE_SELECTOR = '[data-view-id="hotkeys"]';

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
    const ascendIfDeep = useCallback(() => { ascend(); }, []);
    const hk = useHotkeys({ onAfterSave: ascendIfDeep, onAfterClose: ascendIfDeep });
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
            handleKey: hk.handleKey,
            isBlocking: hk.isBlocking,
        });
    }, [hk.editModal, hk.plugins, hk.handleKey, hk.isBlocking]);
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
        { id: 'hotkeys:add', label: 'Add new hotkey', run: () => {
            hkRef.current.openEditModal();
            diveViaSelector(HOTKEYS_EDITOR_DIVE_SELECTOR);
        } },
        { id: 'hotkeys:delete', label: 'Delete selected hotkey', run: () => hkRef.current.deleteSelected() },
        { id: 'hotkeys:edit', label: 'Edit selected hotkey', run: () => {
            const h = filteredRef.current[hkRef.current.selectedIndex];
            if (!h) return;
            hkRef.current.openEditModal(h);
            diveViaSelector(HOTKEYS_EDITOR_DIVE_SELECTOR);
        } },
    ], []);
    useRegisterCommands('hotkeys', commands);

    return html`
        <div class="view-container content-shell">
            <${PageHeader} subtitle="Configure global keyboard shortcuts for plugin actions" />
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

    const slotHandleKey = useCallback((e) => {
        const fn = editSlot.get().handleKey;
        if (fn) fn(e);
    }, []);
    const slotIsBlocking = useCallback(() => {
        const fn = editSlot.get().isBlocking;
        return fn ? fn() : false;
    }, []);
    useRegisterViewKeyboard('hotkeys-editor', slotHandleKey, slotIsBlocking);

    const { modal, plugins, fieldProps, handlers } = editSlot.get();
    if (!modal) {
        return html`<div class="view-container content-shell">
            <${PageHeader} title="Hotkey Editor" subtitle="Select a hotkey to edit" />
        </div>`;
    }
    const canSave = !!(modal.key && modal.pluginId && modal.action);
    return html`
        <div class="view-container content-shell">
            <${PageHeader} title=${modal.hotkey ? 'Edit Hotkey' : 'Add Hotkey'}
                subtitle=${modal.key || 'new hotkey'} />
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
                            <${ModalActions} onClose=${handlers.onClose} onSave=${handlers.onSave} disabled=${!canSave} />
                        </div>
                    <//>
                </div>
            </div>
        </div>
    `;
}
