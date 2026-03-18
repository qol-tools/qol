import { html } from '../../../lib/html.js';
import { useState } from 'preact/hooks';
import { BuildResults } from './BuildResults.js';
import { SELF_UPDATE_EVENT } from '../../../components/app/useSidebarActions.js';

function ReloadCard({ building, buildResults, lastReload, error, reloadPlugins }) {
    return html`
        <div class="dev-card" onClick=${reloadPlugins}>
            <button class=${'refresh-btn ' + (building ? 'spinning' : '')} tabindex="-1" aria-hidden="true"></button>
            <div class="dev-card-content">
                <h3>${building ? 'Building...' : 'Reload All Plugins'}</h3>
                <p>${building ? 'Compiling linked plugins' : 'Build linked plugins and restart daemons.'}</p>
                <${BuildResults} buildResults=${buildResults} />
                ${lastReload && html`<span class="last-action">Last: ${lastReload}</span>`}
                ${error && html`<span class="error-msg">${error}</span>`}
            </div>
            <div class="dev-card-hint"><kbd>Ctrl+r</kbd></div>
        </div>
    `;
}

function MockCard({ mockTesting, triggerMockFlows }) {
    return html`
        <div class=${'dev-card ' + (mockTesting ? 'is-loading' : '')} onClick=${triggerMockFlows}>
            <button class=${'refresh-btn ' + (mockTesting ? 'spinning' : 'is-hidden')} tabindex="-1" aria-hidden="true"></button>
            <div class="dev-card-content">
                <h3>${mockTesting ? 'Stop testing mock flows' : 'Test mock flows'}</h3>
                <p>${mockTesting
                    ? 'Mock progress simulation is running. Click to stop.'
                    : 'Runs all registered mock progress targets without real recompiles.'}</p>
            </div>
        </div>
    `;
}

function SelfUpdateCard() {
    const [result, setResult] = useState(null);
    const [running, setRunning] = useState(false);
    async function dryRun() {
        if (running) return;
        setRunning(true);
        setResult(null);
        try {
            const res = await fetch('/api/dev/test-self-update', { method: 'POST' });
            setResult(await res.json());
        } catch (e) {
            setResult({ ok: false, steps: [{ step: 'request', ok: false, detail: e.message }] });
        }
        setRunning(false);
    }
    async function liveTest(e) {
        e.stopPropagation();
        if (running) return;
        try {
            const res = await fetch('/api/dev/test-self-update?live=1', { method: 'POST' });
            if (!res.ok) {
                setResult({ ok: false, steps: [...(result?.steps || []), { step: 'live', ok: false, detail: 'HTTP ' + res.status }] });
                return;
            }
        } catch {
            setResult({ ok: false, steps: [...(result?.steps || []), { step: 'live', ok: false, detail: 'Failed to configure fixture URL' }] });
            return;
        }
        setResult(null);
        setRunning(false);
        document.dispatchEvent(new Event(SELF_UPDATE_EVENT));
    }
    const showLive = result?.ok && !running;
    return html`
        <div class=${'dev-card' + (result && !result.ok ? ' has-error' : '')} onClick=${dryRun}>
            <div class="dev-card-content">
                <h3>${running ? 'Running...' : 'Test Self-Update'}</h3>
                <p>Dry-run: downloads fixture tarball from itself, extracts, and verifies the binary.</p>
                ${result && html`<ul class="test-update-steps">
                    ${result.steps.map(s => html`<li class=${s.ok ? 'step-ok' : 'step-fail'}>
                        <strong>${s.step}</strong> ${s.ok ? '\u2713' : '\u2717'} ${s.detail}
                    </li>`)}
                </ul>`}
                ${showLive && html`<button class="dev-live-update-btn" onClick=${liveTest}>Install + restart for real</button>`}
            </div>
        </div>
    `;
}

export function ActionsSection({ ctrl }) {
    return html`
        <section class="dev-section">
            <h2>Actions</h2>
            <${ReloadCard}
                building=${ctrl.building}
                buildResults=${ctrl.buildResults}
                lastReload=${ctrl.lastReload}
                error=${ctrl.error}
                reloadPlugins=${ctrl.reloadPlugins}
            />
            <${MockCard} mockTesting=${ctrl.mockTesting} triggerMockFlows=${ctrl.triggerMockFlows} />
            <${SelfUpdateCard} />
        </section>
    `;
}
