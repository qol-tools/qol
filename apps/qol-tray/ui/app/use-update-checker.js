import { useState, useCallback, useEffect } from 'preact/hooks';

async function fetchUpdateResult() {
    const res = await fetch('/api/check-update');
    if (!res.ok) throw new Error();
    return res.json();
}

async function doCheckUpdate(setUpdateState) {
    setUpdateState({ status: 'checking' });
    const minDelay = new Promise(resolve => setTimeout(resolve, 800));
    let result = null;
    try { result = await fetchUpdateResult(); } catch {}
    await minDelay;
    if (!result) { setUpdateState({ status: 'error' }); return; }
    setUpdateState(result.available ? { status: 'available', latest: result.latest } : { status: 'up-to-date' });
}

export function useUpdateChecker(devEnabled, appVersion) {
    const [updateState, setUpdateState] = useState({ status: 'checking' });
    const checkForUpdate = useCallback(() => doCheckUpdate(setUpdateState), []);
    useEffect(() => { if (devEnabled) setUpdateState({ status: 'idle' }); }, [devEnabled]);
    useEffect(() => { if (!devEnabled && appVersion) checkForUpdate(); }, [appVersion, checkForUpdate, devEnabled]);
    return { updateState, setUpdateState, checkForUpdate };
}
