import { html } from '../../lib/html.js';
import { useState, useEffect, useRef, useCallback } from 'preact/hooks';
import { PageHeader } from '../../components/PageHeader.js';
import { SurfaceContainer } from '../../lib/components/SurfaceContainer.js';
import { Button } from '../../lib/components/Button.js';
import { Surface } from '../../lib/components/Surface.js';
import { createSharedSlot } from '../../lib/shared-slot.js';
import { patternsFromInput, patternsToInput } from '../../lib/log-filter-patterns.js';

// TODO: replace with navigation-scoped payload (debt item #7).
export const logFiltersSlot = createSharedSlot({
    scope: null,
    pluginId: null,
    sectionId: null,
    label: '',
    current: [],
    save: null,
});

function dispatchEscape() {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
}

export function LogFiltersSubPage() {
    const [, bump] = useState(0);
    useEffect(() => logFiltersSlot.subscribe(() => bump(t => t + 1)), []);

    const slot = logFiltersSlot.get();
    const [draft, setDraft] = useState(patternsToInput(slot.current));
    const inputRef = useRef(null);

    useEffect(() => {
        setDraft(patternsToInput(logFiltersSlot.get().current));
    }, [slot.scope, slot.pluginId, slot.sectionId]);

    useEffect(() => {
        if (inputRef.current) inputRef.current.focus({ preventScroll: true });
    }, [slot.scope, slot.pluginId, slot.sectionId]);

    const onSave = useCallback(async () => {
        const fn = logFiltersSlot.get().save;
        if (!fn) { dispatchEscape(); return; }
        try {
            await fn(patternsFromInput(draft));
        } finally {
            dispatchEscape();
        }
    }, [draft]);

    const onCancel = useCallback(() => dispatchEscape(), []);

    if (!slot.scope) {
        return html`<div class="view-container content-shell">
            <${PageHeader} title="Edit Log Filters" subtitle="Open from a Dev row" />
        </div>`;
    }

    const subtitle = slot.scope === 'core'
        ? `Core section: ${slot.label || slot.sectionId}`
        : `Plugin: ${slot.label || slot.pluginId}`;

    const onInputKey = (e) => {
        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            onSave();
        }
    };

    return html`
        <div class="view-container content-shell">
            <${PageHeader} title="Edit Log Filters" subtitle=${subtitle} />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame log-filters-frame">
                        <div class="log-filters-form">
                            <p class="log-filters-hint">
                                Mute log lines containing these comma-separated substrings.
                                Leave empty to clear.
                            </p>
                            <${Surface}
                                as="label"
                                className="log-filters-input-row"
                                onActivate=${() => inputRef.current?.focus()}>
                                <input
                                    ref=${inputRef}
                                    type="text"
                                    class="text-input"
                                    value=${draft}
                                    placeholder="error, warn, deprecated"
                                    onInput=${(e) => setDraft(e.currentTarget.value)}
                                    onKeyDown=${onInputKey} />
                            <//>
                            <div class="log-filters-actions">
                                <${Button} variant="btn-ghost" onActivate=${onCancel}>
                                    Cancel <kbd>Esc</kbd>
                                <//>
                                <${Button} variant="btn-primary" onActivate=${onSave}>
                                    Save <kbd>Enter</kbd>
                                <//>
                            </div>
                        </div>
                    <//>
                </div>
            </div>
        </div>
    `;
}
