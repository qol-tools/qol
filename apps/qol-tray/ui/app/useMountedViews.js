import { useEffect, useState } from 'preact/hooks';

export function useMountedViews(activeViewId) {
    const [mounted, setMounted] = useState(() => new Set([activeViewId]));

    useEffect(() => {
        setMounted(prev => {
            if (prev.has(activeViewId)) {
                return prev;
            }
            const next = new Set(prev);
            next.add(activeViewId);
            return next;
        });
    }, [activeViewId]);

    return mounted;
}
