import { html } from '../../../lib/html.js';
import { useState } from 'preact/hooks';
import { Surface } from '../../../lib/components/Surface.js';
import { RefreshButton } from '../../../lib/components/Button.js';
import { BuildResults } from './BuildResults.js';
import { SELF_UPDATE_EVENT } from '../../../app/useSidebarActions.js';
import { diveViaSelector } from '../../../lib/world-navigation-singleton.js';

const GALLERY_DIVE_SELECTOR = '[data-dive-source="dev-component-gallery"]';

function ReloadCard({ building, buildResults, lastReload, error, reloadPlugins }) {
    return html`
        <${Surface} className="dev-card" onActivate=${reloadPlugins}>
            <${RefreshButton} spinning=${building} tabIndex="-1" aria-hidden="true" />
            <div class="dev-card-content">
                <h3>${building ? 'Building...' : 'Reload All Plugins'}</h3>
                <p>${building ? 'Compiling linked plugins' : 'Build linked plugins and restart daemons.'}</p>
                <${BuildResults} buildResults=${buildResults} />
                ${lastReload && html`<span class="last-action">Last: ${lastReload}</span>`}
                ${error && html`<span class="error-msg">${error}</span>`}
            </div>
        <//>
    `;
}

function MockCard({ mockTesting, triggerMockFlows }) {
    return html`
        <${Surface} className=${'dev-card ' + (mockTesting ? 'is-loading' : '')} onActivate=${triggerMockFlows}>
            <${RefreshButton} spinning=${mockTesting} className=${mockTesting ? '' : 'is-hidden'} tabIndex="-1" aria-hidden="true" />
            <div class="dev-card-content">
                <h3>${mockTesting ? 'Stop testing mock flows' : 'Test mock flows'}</h3>
                <p>${mockTesting
                    ? 'Mock progress simulation is running. Click to stop.'
                    : 'Runs all registered mock progress targets without real recompiles.'}</p>
            </div>
        <//>
    `;
}

function SelfUpdateCard() {
    const [running, setRunning] = useState(false);
    const [error, setError] = useState(null);
    async function triggerLiveUpdate() {
        if (running) return;
        setRunning(true);
        setError(null);
        try {
            const setup = await fetch('/api/dev/test-self-update?live=1', { method: 'POST' });
            if (!setup.ok) {
                setError('Failed to configure fixture URL: HTTP ' + setup.status);
                setRunning(false);
                return;
            }
            document.dispatchEvent(new Event(SELF_UPDATE_EVENT));
        } catch (e) {
            setError(e.message);
            setRunning(false);
        }
    }
    return html`
        <${Surface} className=${'dev-card' + (error ? ' has-error' : '')} onActivate=${triggerLiveUpdate}>
            <div class="dev-card-content">
                <h3>${running ? 'Updating...' : 'Test Self-Update'}</h3>
                <p>${running ? 'Installing fixture binary and restarting...' : 'Builds a test fixture from the running binary, installs it, and restarts.'}</p>
                ${error && html`<span class="error-msg">${error}</span>`}
            </div>
        <//>
    `;
}

function ComponentGalleryCard() {
    return html`
        <${Surface} className="dev-card"
            onActivate=${() => diveViaSelector(GALLERY_DIVE_SELECTOR)}>
            <div class="dev-card-content">
                <h3>Component Gallery</h3>
                <p>Browse all UI components and their states.</p>
            </div>
        <//>
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
            <${ComponentGalleryCard} />
        </section>
    `;
}
