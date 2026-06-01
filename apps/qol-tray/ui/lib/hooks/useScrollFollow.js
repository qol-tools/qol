import { useLayoutEffect } from 'preact/hooks';

export function useScrollFollow(containerRef, active, index, selector) {
    useLayoutEffect(() => {
        if (!active) return;
        const items = containerRef.current?.querySelectorAll(selector);
        items?.[index]?.scrollIntoView({ block: 'nearest' });
    }, [active, index, selector]);
}
