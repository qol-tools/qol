import { html } from '../../lib/html.js';
import { PageShell } from '../../components/PageShell.js';
import { ComponentsCatalog } from './components/ComponentsCatalog.js';

export function GalleryShowcasePage({ showcaseId }) {
    return html`
        <${PageShell} frame=${false}>
            <${ComponentsCatalog} activeId=${showcaseId} />
        <//>
    `;
}
