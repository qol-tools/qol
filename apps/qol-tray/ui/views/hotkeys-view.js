import { html } from '../lib/html.js';
import { useRef, useMemo, useCallback } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../app/view-keyboard-context.js';
import { matchesQuery } from '../utils/collections.js';
import { ascend, diveViaSelector } from '../lib/world-navigation-singleton.js';

import { PageHeader } from '../components/PageHeader.js';
import { ModalActions } from '../lib/components/ModalPreact.js';
import { ToggleSwitch } from '../lib/components/ToggleSwitch.js';
import { DiveEditorSubPage } from '../lib/components/DiveEditorSubPage.js';
import { useDiveEditor } from '../lib/hooks/useDiveEditor.js';
import { PageShell } from '../components/PageShell.js';
import { KeyLegend } from '../lib/components/KeyLegend.js';
import { useViewBindings } from '../lib/hooks/useViewBindings.js';
import { PluginSelect, ActionSelect, KeyInput } from './hotkeys/modal.js';
import { useHotkeys } from './hotkeys/use-hotkeys.js';
import { HotkeysList } from './hotkeys/list.js';

import { createSharedSlot } from '../lib/shared-slot.js';
export const hotkeyEditorSlot = createSharedSlot({
    modal: null,
    plugins: [],
    recording: false,
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
    useDiveEditor({
        slot: hotkeyEditorSlot,
        deps: [hk.editModal, hk.plugins, hk.recorder.isRecording, hk.handleKey, hk.isBlocking],
        build: () => ({
            modal: hk.editModal,
            plugins: hk.plugins,
            recording: hk.recorder.isRecording,
            fieldProps: hk.fieldProps,
            handlers: {
                onPluginChange: hk.handlePluginChange,
                onActionChange: hk.handleActionChange,
                onEnabledChange: hk.handleEnabledChange,
                onStartRecording: hk.startRecording,
                onClose: hk.closeModal,
                onSave: hk.saveHotkey,
            },
            handleKey: hk.handleKey,
            isBlocking: hk.isBlocking,
        }),
    });
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

    const bindings = useViewBindings('hotkeys');
    return html`
        <${PageShell}
            subtitle="Configure global keyboard shortcuts for plugin actions"
            aside=${html`<${KeyLegend} bindings=${bindings} />`}>
            ${hk.registrationErrors.length > 0 && html`<${RegistrationWarnings} errors=${hk.registrationErrors} />`}
            <${HotkeysList} hotkeys=${filtered} plugins=${hk.plugins}
                selectedIndex=${hk.selectedIndex} onSelect=${hk.setSelectedIndex} onEdit=${hk.openEditModal} />
        <//>
    `;
}

export function HotkeyEditorSubPage({ slot, viewId = 'hotkeys-editor' }) {
    return html`<${DiveEditorSubPage}
        slot=${slot}
        viewId=${viewId}
        fallbackTitle="Hotkey Editor"
        fallbackSubtitle="Select a hotkey to edit"
        renderHeader=${(v) => html`<${PageHeader}
            title=${v.modal.hotkey ? 'Edit Hotkey' : 'Add Hotkey'}
            subtitle=${v.modal.key || 'new hotkey'} />`}
        children=${(v) => html`<${HotkeyEditorBody} value=${v} />`} />`;
}

function HotkeyEditorBody({ value }) {
    const { modal, plugins, recording, fieldProps, handlers } = value;
    const canSave = !!(modal.key && modal.pluginId && modal.action);
    return html`
        <div class="edit-modal-content">
            <div class="form-group" ...${fieldProps(0)}>
                <${ToggleSwitch} checked=${modal.enabled !== false}
                    onChange=${handlers.onEnabledChange}
                    label="Active" />
            </div>
            <div class="form-group" ...${fieldProps(1)}>
                <label>Plugin</label>
                <${PluginSelect} modal=${modal} plugins=${plugins} onChange=${handlers.onPluginChange} />
            </div>
            <div class="form-group" ...${fieldProps(2)}>
                <label>Action</label>
                <${ActionSelect} modal=${modal} onChange=${handlers.onActionChange}
                    disabled=${modal.availableActions.length === 0} />
            </div>
            <div class="form-group" ...${fieldProps(3)}>
                <label>Shortcut</label>
                <${KeyInput} modal=${modal} recording=${recording} onStartRecording=${handlers.onStartRecording} />
            </div>
            <${ModalActions} onClose=${handlers.onClose} onSave=${handlers.onSave} disabled=${!canSave} />
        </div>
    `;
}
