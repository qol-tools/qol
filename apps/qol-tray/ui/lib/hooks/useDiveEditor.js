import { useEffect } from 'preact/hooks';

/**
 * Glue hook for editor parent views: pushes a payload into a createSharedSlot
 * whenever the relevant inputs change. The DiveEditorSubPage subscribes to the
 * same slot and renders the editor body.
 *
 * Usage:
 *     useDiveEditor({
 *         slot: editSlot,
 *         deps: [hk.editModal, hk.handleKey, hk.isBlocking],
 *         build: () => ({
 *             modal: hk.editModal,
 *             plugins: hk.plugins,
 *             fieldProps: hk.fieldProps,
 *             handlers: { ... },
 *             handleKey: hk.handleKey,
 *             isBlocking: hk.isBlocking,
 *         }),
 *     });
 */
export function useDiveEditor({ slot, build, deps }) {
    useEffect(() => {
        slot.set(build());
    }, deps);
}
