import { useEffect, useCallback } from 'preact/hooks';
import { useStateRef } from '../../hooks/useStateRef.js';
import { usePersistedIndex } from '../../hooks/usePersistedIndex.js';
import {
    buildApiExample,
    loadTaskRunnerData,
    nextSelectedIndex,
    persistTaskRunnerConfig,
    removeSelectedAction
} from './data.js';

async function doLoad(setActions, setActionIds, setSelectedIndex, markRestored) {
    try {
        const loaded = await loadTaskRunnerData();
        setActions(loaded.actions);
        setActionIds(loaded.actionIds);
        setSelectedIndex(prev => {
            markRestored();
            return prev >= 0 && prev < loaded.actionIds.length ? prev : 0;
        });
    } catch {}
}

function doDelete(actionsRef, actionIdsRef, selectedIndexRef, setActions, setActionIds, setSelectedIndex) {
    const ids = actionIdsRef.current;
    const idx = selectedIndexRef.current;
    if (ids.length === 0 || idx < 0) return;
    const nextActions = removeSelectedAction(actionsRef.current, ids, idx);
    const nextIds = Object.keys(nextActions);
    setActions(nextActions);
    setActionIds(nextIds);
    setSelectedIndex(nextSelectedIndex(nextIds, idx));
    void persistTaskRunnerConfig(nextActions);
}

export function useTaskData() {
    const [actions, setActions, actionsRef] = useStateRef({});
    const [actionIds, setActionIds, actionIdsRef] = useStateRef([]);
    const [si, setSI, siRef, markRestored] = usePersistedIndex('taskrunner-selected-index');
    const loadActions = useCallback(
        () => doLoad(setActions, setActionIds, setSI, markRestored),
        []
    );
    useEffect(() => { loadActions(); }, [loadActions]);
    const deleteAction = useCallback(
        () => doDelete(actionsRef, actionIdsRef, siRef, setActions, setActionIds, setSI),
        []
    );
    const copyApiExample = useCallback(
        () => navigator.clipboard.writeText(buildApiExample(actionsRef.current, actionIdsRef.current).json),
        []
    );
    return { actions, setActions, actionsRef, actionIds, setActionIds, actionIdsRef, selectedIndex: si, setSelectedIndex: setSI, selectedIndexRef: siRef, deleteAction, copyApiExample };
}
