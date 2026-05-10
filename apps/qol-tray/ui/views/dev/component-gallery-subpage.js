import { html } from '../../lib/html.js';
import { useState } from 'preact/hooks';
import { PageHeader } from '../../components/PageHeader.js';
import { SurfaceContainer } from '../../lib/components/SurfaceContainer.js';
import { Surface } from '../../lib/components/Surface.js';
import { ComponentsCatalog, SHOWCASE_KEYS } from './components/ComponentsCatalog.js';

export function ComponentGallerySubPage() {
    const [activeId, setActiveId] = useState(SHOWCASE_KEYS[0]);

    return html`
        <div class="view-container content-shell">
            <${PageHeader} title="Component Gallery" />
            <${SurfaceContainer} className="view-body">
                <div class="gallery-layout">
                    <${SurfaceContainer} className="gallery-nav">
                        ${SHOWCASE_KEYS.map(id => html`
                            <${Surface} key=${id}
                                className=${'gallery-nav-item' + (id === activeId ? ' is-active' : '')}
                                onActivate=${() => setActiveId(id)}>
                                ${id}
                            <//>
                        `)}
                    <//>
                    <div class="gallery-content">
                        <${ComponentsCatalog} activeId=${activeId} />
                    </div>
                </div>
            <//>
        </div>
    `;
}
