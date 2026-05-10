import { html } from '../../lib/html.js';
import { PageHeader } from '../../components/PageHeader.js';
import { SurfaceContainer } from '../../lib/components/SurfaceContainer.js';
import { ComponentsCatalog } from './components/ComponentsCatalog.js';

export function GalleryShowcasePage({ showcaseId }) {
    return html`
        <div class="view-container content-shell">
            <${PageHeader} title=${showcaseId} />
            <${SurfaceContainer} className="view-body">
                <${ComponentsCatalog} activeId=${showcaseId} />
            <//>
        </div>
    `;
}
