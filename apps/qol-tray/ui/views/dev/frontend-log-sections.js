import { isDebugEnabled, setDebugEnabled } from '../../lib/debug.js';

export const FRONTEND_LOG_SECTIONS = [
    {
        id: 'frontend-debug',
        name: 'Frontend Debug',
        description: 'Console logging for UI navigation, focus, surface',
        isMuted: () => !isDebugEnabled(),
        setMuted: muted => setDebugEnabled(!muted)
    }
];

export function findFrontendLogSection(sectionId) {
    return FRONTEND_LOG_SECTIONS.find(section => section.id === sectionId) || null;
}
