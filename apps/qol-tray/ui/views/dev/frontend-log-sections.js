import { isDebugEnabled, setDebugEnabled } from '../../lib/debug.js';

// Frontend-only log sections — synthetic entries injected into the
// core-log-controls payload. The backend doesn't track these; mute state
// lives client-side (e.g. localStorage via debug.js). Add a new entry here
// to surface another frontend-only knob in the dev log UI.
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
