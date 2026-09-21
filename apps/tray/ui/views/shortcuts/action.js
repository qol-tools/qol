export const TYPE_LABELS = { open_url: 'URL', launch_app: 'App', plugin_action: 'Plugin' };

export function actionSummary(action) {
    switch (action?.type) {
        case 'open_url': return action.url || '(no url)';
        case 'launch_app': return appRefLabel(action.app);
        case 'plugin_action': return `${action.plugin_id || ''} / ${action.action || ''}`.trim();
        default: return '(unknown)';
    }
}

export function actionSearchText(action) {
    switch (action?.type) {
        case 'open_url': return action.url || '';
        case 'launch_app': return Object.values(action.app || {}).join(' ');
        case 'plugin_action': return `${action.plugin_id || ''} ${action.action || ''}`;
        default: return '';
    }
}

export function isManagedPluginShortcut(shortcut) {
    return shortcut?.source?.type === 'plugin_manifest' || shortcut?.action?.type === 'plugin_action';
}

function appRefLabel(appRef) {
    switch (appRef?.type) {
        case 'bundle_id': return appRef.id;
        case 'path': return appRef.path;
        case 'name': return appRef.name;
        default: return '';
    }
}
