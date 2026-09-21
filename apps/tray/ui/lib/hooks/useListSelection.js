import { useCallback, useState } from 'preact/hooks';

export function useListSelection(initial = -1) {
    const [index, setIndex] = useState(initial);
    const deselect = useCallback(() => setIndex(-1), []);
    const selected = useCallback((i) => i === index, [index]);
    return { index, select: setIndex, deselect, selected };
}
