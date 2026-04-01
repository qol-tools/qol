import { html } from '../lib/html.js';
import { useCallback } from 'preact/hooks';
import { useViewTabs } from '../hooks/useViewTabs.js';
import { PageHeader } from './PageHeader.js';
import { SurfaceContainer } from './SurfaceContainer.js';

/**
 * Full tabbed view shell. Renders page header, content frame, tab bar,
 * and the active tab's content panel inside a SurfaceContainer.
 *
 * Usage:
 *   <${ViewTabs} title="Logs" subtitle="..." tabs=${TABS} vtRef=${vtRef}>
 *       ${(vt) => html`
 *           ${vt.activeTab === 'live' && html`<${LiveLog} />`}
 *           ${vt.activeTab === 'suppressed' && html`<${SuppressedList} />`}
 *       `}
 *   <//>
 */
export function ViewTabs({ title, subtitle, scramble, tabs, onActivate, onContentBlur, trailing, children, vtRef, className, containerRef }) {
    const vt = useViewTabs(tabs, { onActivate });

    if (vtRef) vtRef.current = vt;

    const shellClass = ['view-container content-shell', className].filter(Boolean).join(' ');

    const handleContentFocusOut = useCallback((e) => {
        if (!onContentBlur) return;
        const content = e.currentTarget;
        if (!e.relatedTarget || !content.contains(e.relatedTarget)) {
            if (e.relatedTarget?.closest('.edit-modal, .confirm-modal')) return;
            onContentBlur();
        }
    }, [onContentBlur]);

    return html`
        <div class=${shellClass} ref=${containerRef}>
            <${PageHeader} title=${title} subtitle=${subtitle} scramble=${scramble} />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame">
                        <div class="view-tabs" role="tablist" ref=${vt.rootRef}>
                            ${tabs.map((tab, i) => html`
                                <button
                                    key=${tab.id}
                                    class="view-tab ${vt.activeTab === tab.id ? 'active' : ''}"
                                    role="tab"
                                    data-selected-surface=""
                                    data-selected=${vt.activeTab === tab.id ? 'true' : 'false'}
                                    data-tab-id=${tab.id}
                                    aria-selected=${vt.activeTab === tab.id}
                                    onClick=${() => vt.activateTab(i)}
                                >
                                    ${tab.label}
                                    ${tab.count > 0 ? html`<span class="view-tab-count">${tab.count}</span>` : null}
                                </button>
                            `)}
                            ${trailing}
                        </div>
                        <${SurfaceContainer} className="view-tab-content" role="tabpanel"
                            onFocusOut=${handleContentFocusOut}>
                            ${typeof children === 'function' ? children(vt) : children}
                        <//>
                    <//>
                </div>
            </div>
        </div>
    `;
}
