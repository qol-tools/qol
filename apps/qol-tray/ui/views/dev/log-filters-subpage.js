import { html } from '../../lib/html.js';
import { useState, useEffect, useRef, useCallback } from 'preact/hooks';
import { PageHeader } from '../../components/PageHeader.js';
import { SurfaceContainer } from '../../lib/components/SurfaceContainer.js';
import { Button } from '../../lib/components/Button.js';
import { Surface } from '../../lib/components/Surface.js';
import { createSharedSlot } from '../../lib/shared-slot.js';
import { patternsFromInput, patternsToInput } from '../../lib/log-filter-patterns.js';

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

export function LogFiltersSubPage({ slot }) {
    const [, bump] = useState(0);
    useEffect(() => slot.subscribe(() => bump(t => t + 1)), [slot]);

    const value = slot.get();
    const [draft, setDraft] = useState(patternsToInput(value.current));
    const inputRef = useRef(null);

    useEffect(() => {
        setDraft(patternsToInput(slot.get().current));
    }, [value.scope, value.pluginId, value.sectionId]);

    useEffect(() => {
        if (inputRef.current) inputRef.current.focus({ preventScroll: true });
    }, [value.scope, value.pluginId, value.sectionId]);

    const onSave = useCallback(async () => {
        const fn = slot.get().save;
        if (!fn) { dispatchEscape(); return; }
        try {
            await fn(patternsFromInput(draft));
        } finally {
            dispatchEscape();
        }
    }, [slot, draft]);

    const onCancel = useCallback(() => dispatchEscape(), []);

    if (!value.scope) {
        return html`<div class="view-container content-shell">
            <${PageHeader} title="Edit Log Filters" subtitle="Open from a Dev row" />
        </div>`;
    }

    const subtitle = value.scope === 'core'
        ? `Core section: ${value.label || value.sectionId}`
        : `Plugin: ${value.label || value.pluginId}`;

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
