import { html } from '../../lib/html.js';
import { useCallback, useState } from 'preact/hooks';
import { createSharedSlot } from '../../lib/shared-slot.js';
import { useDiveEditor } from '../../lib/hooks/useDiveEditor.js';
import { useModalKeyboard } from '../../lib/hooks/useModalKeyboard.js';
import { ascend } from '../../lib/world-navigation-singleton.js';
import { HotkeyEditorSubPage } from '../hotkeys-view.js';
import { changeEditModalPlugin, createEditModalState } from '../hotkeys/modal.js';

const GALLERY_PLUGINS = [
    { id: 'qol-alt-tab', name: 'Alt Tab', actions: [
        { id: 'open-switcher', label: 'Open switcher' },
        { id: 'next', label: 'Next window' },
        { id: 'prev', label: 'Previous window' },
    ] },
    { id: 'qol-launcher', name: 'Launcher', actions: [
        { id: 'open', label: 'Open launcher' },
    ] },
    { id: 'qol-lights', name: 'Lights', actions: [
        { id: 'toggle', label: 'Toggle lights' },
        { id: 'brighter', label: 'Brighter' },
    ] },
];

const galleryGetActions = (pluginId) => GALLERY_PLUGINS.find(p => p.id === pluginId)?.actions ?? [];

const SAMPLE_HOTKEY = {
    id: 'gallery-hotkey-1',
    plugin_id: 'qol-alt-tab',
    action: 'open-switcher',
    key: 'Alt+Tab',
};

const fallbackFieldProps = (index) => ({
    'data-selected-surface': '',
    'data-selected': index === 0 ? 'true' : 'false',
    tabIndex: -1,
});

export const galleryHotkeyEditorSlot = createSharedSlot({
    modal: createEditModalState(SAMPLE_HOTKEY, null, galleryGetActions),
    plugins: GALLERY_PLUGINS,
    recording: false,
    fieldProps: fallbackFieldProps,
    handlers: {},
    handleKey: null,
    isBlocking: () => false,
});

export function useGalleryHotkeyEditorController() {
    const [modal, setModal] = useState(() => createEditModalState(SAMPLE_HOTKEY, null, galleryGetActions));
    const onPluginChange = useCallback((id) => {
        setModal(prev => prev ? changeEditModalPlugin(prev, id, galleryGetActions) : prev);
    }, []);
    const onActionChange = useCallback((action) => {
        setModal(prev => prev ? { ...prev, action } : prev);
    }, []);
    const onClose = useCallback(() => ascend(), []);
    const onSave = useCallback(() => ascend(), []);
    const { fieldProps } = useModalKeyboard({ onSave, onClose });

    useDiveEditor({
        slot: galleryHotkeyEditorSlot,
        deps: [modal, fieldProps],
        build: () => ({
            modal,
            plugins: GALLERY_PLUGINS,
            recording: false,
            fieldProps,
            handlers: {
                onPluginChange,
                onActionChange,
                onStartRecording: () => {},
                onClose,
                onSave,
            },
            handleKey: null,
            isBlocking: () => false,
        }),
    });

    return {
        open: useCallback((hotkey) => {
            setModal(createEditModalState(hotkey, null, galleryGetActions));
        }, []),
    };
}

export function GalleryHotkeyEditorSubPage() {
    return html`<${HotkeyEditorSubPage} slot=${galleryHotkeyEditorSlot} />`;
}
