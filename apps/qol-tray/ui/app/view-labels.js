export const VIEW_LABELS = {
    plugins: 'Plugins',
    store: 'Plugin Store',
    hotkeys: 'Hotkeys',
    shortcuts: 'Shortcuts',
    'task-runner': 'Task Runner',
    profile: 'Profile',
    logs: 'Logs',
    dev: { text: 'Developer', animation: 'scramble' },
};

export function getViewLabel(id) {
    const entry = VIEW_LABELS[id];
    if (entry == null) return { text: id, animation: null };
    if (typeof entry === 'string') return { text: entry, animation: null };
    return { text: entry.text || id, animation: entry.animation || null };
}

export function resolveViewLabel(entry) {
    if (!entry) return { text: '', animation: null };
    const declared = VIEW_LABELS[entry.id];
    if (declared != null) {
        if (typeof declared === 'string') return { text: declared, animation: null };
        return { text: declared.text || entry.label || entry.id, animation: declared.animation || null };
    }
    if (entry.label) return { text: entry.label, animation: null };
    return { text: entry.id, animation: null };
}
