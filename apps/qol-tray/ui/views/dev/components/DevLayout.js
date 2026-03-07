import { html } from '../../../lib/html.js';
import { PageHeader } from '../../../components/PageHeader.js';
import { PluginsSection } from './PluginsSection.js';
import { CoreLogSection } from './CoreLogSection.js';
import { ActionsSection } from './ActionsSection.js';

export function DevLayout({ ctrl }) {
    return html`
        <div class="view-container dev-view-shell">
            <${PageHeader} title="Developer Control" scramble />
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
