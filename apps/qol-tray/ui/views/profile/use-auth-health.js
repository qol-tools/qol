import { useCallback, useEffect, useState } from 'preact/hooks';
import { fetchAuthHealth } from '../../features/auth/actions.js';

const EMPTY_HEALTH = { issues: [] };

export function useAuthHealth() {
    const [authHealth, setAuthHealth] = useState(EMPTY_HEALTH);

    const refreshAuthHealth = useCallback(async () => {
        try {
            const result = await fetchAuthHealth();
            setAuthHealth(result || EMPTY_HEALTH);
        } catch (_) {
            setAuthHealth(EMPTY_HEALTH);
        }
    }, []);

    useEffect(() => {
        refreshAuthHealth();
    }, [refreshAuthHealth]);

    return { authHealth, refreshAuthHealth };
}
