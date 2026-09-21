function appRef(params) {
    const type = params.app_type || 'name';
    const key = type === 'bundle_id' ? 'id' : type;
    return { type, [key]: params.app || '' };
}

export function buildShortcutPrefill(params = {}) {
    const name = params.name || '';
    const base = { name, enabled: true, export_to_launcher: true };
    if (params.type === 'app') {
        return { editing: false, shortcut: { ...base, action: { type: 'launch_app', app: appRef(params) } } };
    }
    return { editing: false, shortcut: { ...base, action: { type: 'open_url', url: params.url || '' } } };
}
