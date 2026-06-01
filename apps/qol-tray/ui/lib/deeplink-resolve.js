import { buildShortcutPrefill } from '../views/shortcuts/prefill.js';

export function resolveDeepLink(route, deps) {
    if (!route || !route.page) return false;
    if (route.page === 'shortcuts' && route.action === 'add') {
        deps.setPendingShortcutPrefill(buildShortcutPrefill(route.params));
        return true;
    }
    return false;
}
