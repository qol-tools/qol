import { html } from '../../lib/html.js';
import { PageHeader } from '../../components/PageHeader.js';
import { ComponentsCatalog } from './components/ComponentsCatalog.js';

export function ComponentGallerySubPage() {
    return html`
        <div class="view-container content-shell">
            <${PageHeader} title="Component Gallery" subtitle="All UI components and their states" />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${ComponentsCatalog} />
                </div>
            </div>
        </div>
    `;
}
