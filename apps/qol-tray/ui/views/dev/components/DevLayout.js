import { html } from '../../../lib/html.js';
import { PluginsSection } from './PluginsSection.js';
import { CoreLogSection } from './CoreLogSection.js';
import { ActionsSection } from './ActionsSection.js';

function DevPageHeader() {
    return html`
        <div class="page-header dev-stage-head">
            <div class="page-header-main dev-stage-title">
                <h1>Developer Control</h1>
                <p>Link plugins, run rebuild flows, and inspect live runtime state.</p>
            </div>
            <div class="page-header-actions dev-stage-tags" aria-hidden="true">
                <span>Runtime</span>
                <span>Build</span>
                <span>Discovery</span>
            </div>
        </div>
    `;
}

export function DevLayout({ ctrl }) {
    return html`
        <div class="view-container dev-view-shell">
            <${DevPageHeader} />
            <div class="view-body dev-view-body">
                <div class="dev-view-content">
                    <div class="dev-content-frame">
                        <${PluginsSection} ctrl=${ctrl} />
                        <${CoreLogSection} ctrl=${ctrl} />
                        <${ActionsSection} ctrl=${ctrl} />
                    </div>
                </div>
            </div>
        </div>
    `;
}
