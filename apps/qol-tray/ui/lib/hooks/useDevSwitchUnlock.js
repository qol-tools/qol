import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { createUnlockTracker } from '../dev-switch-unlock.js';

export function useDevSwitchUnlock() {
    const trackerRef = useRef(null);
    if (trackerRef.current === null) trackerRef.current = createUnlockTracker();

    const [revealed, setRevealed] = useState(() => trackerRef.current.isRevealed());

    useEffect(() => trackerRef.current.subscribe(setRevealed), []);

    const bumpClick = useCallback(() => trackerRef.current.bumpClick(), []);
    const feedKey = useCallback((key) => trackerRef.current.feedKey(key), []);

    return { revealed, bumpClick, feedKey };
}
