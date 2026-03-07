import { useEffect } from 'preact/hooks';
import { registerCommands, unregisterCommands } from './registry.js';

export function useRegisterCommands(viewId, commands) {
    useEffect(() => {
        registerCommands(viewId, commands);
        return () => unregisterCommands(viewId);
    }, [viewId, commands]);
}
