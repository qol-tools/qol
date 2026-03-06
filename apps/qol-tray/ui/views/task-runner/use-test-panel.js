import { useCallback } from 'preact/hooks';
import { useStateRef } from '../../hooks/useStateRef.js';
import { runTaskActionTest } from './data.js';

async function doRunTest(testingId, testRunning, testParams, setTestRunning, setTestResult) {
    if (!testingId || testRunning) return;
    setTestRunning(true);
    setTestResult(null);
    try {
        setTestResult(await runTaskActionTest(testingId, testParams));
    } catch (e) {
        setTestResult({ success: false, error: e.message, exitCode: -1 });
    }
    setTestRunning(false);
}

export function useTestPanel() {
    const [testingId, setTestingId, testingIdRef] = useStateRef(null);
    const [testParams, setTestParams] = useStateRef({});
    const [testResult, setTestResult] = useStateRef(null);
    const [testRunning, setTestRunning] = useStateRef(false);
    const openTestPanel = useCallback((actionId) => {
        setTestingId(actionId);
        setTestParams({});
        setTestResult(null);
        setTestRunning(false);
    }, []);
    const closeTestPanel = useCallback(() => {
        setTestingId(null);
        setTestParams({});
        setTestResult(null);
    }, []);
    const runTest = useCallback(
        () => doRunTest(testingId, testRunning, testParams, setTestRunning, setTestResult),
        [testingId, testRunning, testParams]
    );
    return { testingId, testingIdRef, testParams, setTestParams, testResult, testRunning, openTestPanel, closeTestPanel, runTest };
}
