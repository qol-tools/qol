import { html } from '../lib/html.js';
import { useCallback } from 'preact/hooks';
import { useViewTabs } from '../lib/hooks/useViewTabs.js';
import { KEYBOARD_ISOLATION_SELECTOR } from '../lib/surface-traits.js';
import { Surface } from '../lib/components/Surface.js';
import { SurfaceContainer } from '../lib/components/SurfaceContainer.js';
import { PageShell } from './PageShell.js';

export function ViewTabs({ subtitle, tabs, onActivate, onContentBlur, trailing, children, vtRef, className, initialTab }) {
    const vt = useViewTabs(tabs, { onActivate, initialTab });

    if (vtRef) vtRef.current = vt;

    const handleContentFocusOut = useCallback((e) => {
        if (!onContentBlur) return;
        const content = e.currentTarget;
        if (!e.relatedTarget || !content.contains(e.relatedTarget)) {
            if (e.relatedTarget?.closest(KEYBOARD_ISOLATION_SELECTOR)) return;
            onContentBlur();
        }
    }, [onContentBlur]);

    return html`
        <${PageShell} subtitle=${subtitle} className=${className}>
            <div class="view-tabs" role="tablist" ref=${vt.rootRef}>
                ${tabs.map((tab, i) => html`
                    <${Surface} as="button" key=${tab.id}
                        className="view-tab ${vt.activeTab === tab.id ? 'active' : ''}"
                        role="tab"
                        selected=${vt.activeTab === tab.id}
                        data-tab-id=${tab.id}
                        aria-selected=${vt.activeTab === tab.id}
                        onSelect=${() => vt.previewTab(i)}
                        onActivate=${() => vt.activateTab(i)}>
                        ${tab.label}
                        ${tab.count > 0 ? html`<span class="view-tab-count">${tab.count}</span>` : null}
                    <//>
                `)}
                ${trailing}
            </div>
            <${SurfaceContainer} className="view-tab-content" role="tabpanel"
                onFocusOut=${handleContentFocusOut}>
                ${typeof children === 'function' ? children(vt) : children}
            <//>
        <//>
    `;
}
