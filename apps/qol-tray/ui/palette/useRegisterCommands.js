import { useEffect, useRef } from 'preact/hooks';
import { registerCommands, unregisterCommands } from './registry.js';

export function useRegisterCommands(viewId, commands) {
    const scopeRef = useRef(null);
    if (!scopeRef.current) scopeRef.current = Symbol('cmd-scope');
    useEffect(() => {
        registerCommands(viewId, scopeRef.current, commands);
        return () => unregisterCommands(viewId, scopeRef.current);
    }, [viewId, commands]);
}
