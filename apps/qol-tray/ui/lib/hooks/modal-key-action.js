// Pure dispatch decision for useModalKeyboard.handleKey. Extracted so the
// contract can be tested without DOM/preact deps.
//
// Returns one of:
//   'noop'                — no action, don't preventDefault
//   'blur-edit'           — return focus from the editing input to its surface
//   'blur-edit-and-save'  — blur edit + save (Ctrl+Enter while editing)
//   'save'                — save (Ctrl+Enter on a surface)
//   'close'               — close modal (Escape on a surface)
//
// Esc-on-surface MUST resolve to 'close' when an onClose handler is provided.
// If it resolves to 'noop', globalSurfaceNav.ascendLayer() runs the dive
// ascend without ever clearing the parent view's editModal, the parent's
// isBlocking() stays true, and root-layer Tab cycling silently breaks.
export function resolveModalKeyAction({ key, ctrlKey, isEditing, hasOnClose }) {
    if (isEditing) {
        if (key === 'Enter' && ctrlKey) return 'blur-edit-and-save';
        if (key === 'Escape' || key === 'Enter') return 'blur-edit';
        return 'noop';
    }
    if (key === 'Enter' && ctrlKey) return 'save';
    if (key === 'Escape' && hasOnClose) return 'close';
    return 'noop';
}
