import { useState, useCallback, useEffect } from 'preact/hooks';

function readSubPath(viewId) {
    const raw = location.hash.replace(/^#/, '');
    const segments = raw.split('/').filter(Boolean);
    if (segments[0] !== viewId) return [];
    return segments.slice(1);
}

export function useHashSubPath(viewId) {
    const [subPath, setSubPathState] = useState(() => readSubPath(viewId));

    const setSubPath = useCallback((segments) => {
        setSubPathState(segments);
        const currentRaw = location.hash.replace(/^#/, '');
        const currentView = currentRaw.split('/')[0];
        if (currentView !== viewId) return;
        const path = segments.length ? `${viewId}/${segments.join('/')}` : viewId;
        history.replaceState(null, '', `#${path}`);
    }, [viewId]);

    useEffect(() => {
        const onHashChange = () => setSubPathState(readSubPath(viewId));
        window.addEventListener('hashchange', onHashChange);
        return () => window.removeEventListener('hashchange', onHashChange);
    }, [viewId]);

    return [subPath, setSubPath];
}
