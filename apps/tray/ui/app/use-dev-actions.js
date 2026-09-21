import { useCallback } from 'preact/hooks';

export function useDevActions(devEnabled, devFlows, setUpdateState) {
    const beginSelfUpdate = useCallback(() => {
        if (devEnabled) { devFlows.beginUpdateFlow(); return; }
        setUpdateState({ status: 'downloading', percent: 0 });
    }, [devEnabled, devFlows.beginUpdateFlow, setUpdateState]);
    const failSelfUpdate = useCallback(() => {
        if (devEnabled) { devFlows.applyDevFlowTransition('update', 'failed', { message: 'Update failed' }); return; }
        setUpdateState({ status: 'error' });
    }, [devEnabled, devFlows.applyDevFlowTransition, setUpdateState]);
    const beginDevRecompile = useCallback(() => {
        if (!devEnabled) return false;
        return devFlows.beginRecompileFlow();
    }, [devEnabled, devFlows.beginRecompileFlow]);
    const failDevRecompile = useCallback(
        msg => devFlows.applyDevFlowTransition('recompile', 'failed', { message: msg }),
        [devFlows.applyDevFlowTransition]
    );
    return { beginSelfUpdate, failSelfUpdate, beginDevRecompile, failDevRecompile };
}
