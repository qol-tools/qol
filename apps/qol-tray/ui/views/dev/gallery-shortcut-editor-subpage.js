import { html } from '../../lib/html.js';
import { useCallback, useState } from 'preact/hooks';
import { createSharedSlot } from '../../lib/shared-slot.js';
import { useDiveEditor } from '../../lib/hooks/useDiveEditor.js';
import { useModalKeyboard } from '../../lib/hooks/useModalKeyboard.js';
import { ascend } from '../../lib/world-navigation-singleton.js';
import { ShortcutEditorSubPage } from '../shortcuts-view.js';

const SAMPLE_SHORTCUT = {
    id: 'github',
    name: 'GitHub',
    enabled: true,
    export_to_launcher: true,
    action: { type: 'open_url', url: 'https://github.com' },
};

const fallbackFieldProps = (index) => ({
    'data-selected-surface': '',
    'data-selected': index === 0 ? 'true' : 'false',
    tabIndex: -1,
});

export const galleryShortcutEditorSlot = createSharedSlot({
    modal: { editing: true, shortcut: SAMPLE_SHORTCUT },
    fieldProps: fallbackFieldProps,
    handlers: {},
    handleKey: null,
    isBlocking: () => false,
});

export function useGalleryShortcutEditorController() {
    const [modal, setModal] = useState(() => ({ editing: true, shortcut: SAMPLE_SHORTCUT }));
    const onChange = useCallback((shortcut) => {
        setModal(prev => prev ? { ...prev, shortcut } : prev);
    }, []);
    const onClose = useCallback(() => ascend(), []);
    const onSave = useCallback(() => ascend(), []);
    const { fieldProps, handleKey } = useModalKeyboard({ onSave, onClose });

    useDiveEditor({
        slot: galleryShortcutEditorSlot,
        deps: [modal, fieldProps, handleKey],
        build: () => ({
            modal,
            fieldProps,
            handlers: { onChange, onClose, onSave },
            handleKey,
            isBlocking: () => !!modal,
        }),
    });

    return {
        open: useCallback((shortcut) => {
            setModal({ editing: true, shortcut });
        }, []),
    };
}

export function GalleryShortcutEditorSubPage() {
    return html`<${ShortcutEditorSubPage} slot=${galleryShortcutEditorSlot} viewId="dev-gallery-shortcut-row-editor" />`;
}
