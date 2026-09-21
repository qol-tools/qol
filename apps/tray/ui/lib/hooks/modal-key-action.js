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
