import { useCallback, useState } from 'preact/hooks';

export function useDispatchAction(pluginId, actionName) {
    const [state, setState] = useState({ pending: false, error: null, result: null });

    const dispatch = useCallback(async () => {
        if (!pluginId || !actionName) {
            return null;
        }
        setState({ pending: true, error: null, result: null });
        try {
            const response = await fetch(
                `/api/plugins/${encodeURIComponent(pluginId)}/actions/${encodeURIComponent(actionName)}`,
                {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: '{}',
                    qolSuppressErrorToast: true,
                },
            );
            if (!response.ok) {
                const text = await response.text();
                throw new Error(actionErrorMessage(response.status, text));
            }
            const result = await response.json().catch(() => null);
            setState({ pending: false, error: null, result });
            return result;
        } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            setState({ pending: false, error: message, result: null });
            throw error;
        }
    }, [pluginId, actionName]);

    return { dispatch, pending: state.pending, error: state.error, result: state.result };
}

export function actionErrorMessage(status, text) {
    const trimmed = (text || '').trim();
    if (!trimmed) {
        return `HTTP ${status}`;
    }
    try {
        const body = JSON.parse(trimmed);
        if (typeof body.message === 'string' && body.message.trim()) {
            return body.message.trim();
        }
        if (typeof body.error === 'string' && body.error.trim()) {
            return body.error.trim();
        }
    } catch {}
    return trimmed;
}
