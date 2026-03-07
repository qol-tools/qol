import { html } from '../../../lib/html.js';
import { BuildResults } from './BuildResults.js';

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
        </section>
    `;
}
