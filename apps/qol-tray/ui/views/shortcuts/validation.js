export function isShortcutValid(shortcut) {
    if (!shortcut) return false;
    if (!isNonBlank(shortcut.id)) return false;
    if (!isNonBlank(shortcut.name)) return false;
    return isActionValid(shortcut.action);
}

function isActionValid(action) {
    if (!action) return false;
    if (action.type === 'open_url') {
        if (!isUrlValid(action.url)) return false;
        if (action.browser_override && !isAppRefValid(action.browser_override)) return false;
        return true;
    }
    if (action.type === 'launch_app') return isAppRefValid(action.app);
    return false;
}

function isUrlValid(url) {
    if (!isNonBlank(url)) return false;
    return url.startsWith('http://') || url.startsWith('https://');
}

function isAppRefValid(ref) {
    if (!ref) return false;
    if (ref.type === 'bundle_id') return isNonBlank(ref.id);
    if (ref.type === 'path') return isNonBlank(ref.path);
    if (ref.type === 'name') return isNonBlank(ref.name);
    return false;
}

function isNonBlank(value) {
    return typeof value === 'string' && value.trim().length > 0;
}
