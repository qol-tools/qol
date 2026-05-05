import { html } from '../../../lib/html.js';
import { useEffect, useState, useCallback } from 'preact/hooks';

const ENDPOINT = '/api/dev/tooling-gh-account';

async function loadCurrent() {
    const res = await fetch(ENDPOINT);
    if (!res.ok) throw new Error('HTTP ' + res.status);
    const body = await res.json();
    return body?.value || '';
}

async function persist(value) {
    const res = await fetch(ENDPOINT, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ value: value === null ? null : String(value) }),
    });
    if (!res.ok) throw new Error('HTTP ' + res.status);
}

export function ToolingGhAccountSection() {
    const [draft, setDraft] = useState('');
    const [saved, setSaved] = useState('');
    const [error, setError] = useState(null);
    const [status, setStatus] = useState('idle');

    useEffect(() => {
        let cancelled = false;
        loadCurrent()
            .then((value) => {
                if (cancelled) return;
                setDraft(value);
                setSaved(value);
            })
            .catch((e) => {
                if (cancelled) return;
                setError(e.message);
            });
        return () => { cancelled = true; };
    }, []);

    const onSave = useCallback(async () => {
        setStatus('saving');
        setError(null);
        try {
            const trimmed = draft.trim();
            await persist(trimmed === '' ? null : trimmed);
            setSaved(trimmed);
            setDraft(trimmed);
            setStatus('saved');
        } catch (e) {
            setError(e.message);
            setStatus('idle');
        }
    }, [draft]);

    const onClear = useCallback(async () => {
        setStatus('saving');
        setError(null);
        try {
            await persist(null);
            setDraft('');
            setSaved('');
            setStatus('saved');
        } catch (e) {
            setError(e.message);
            setStatus('idle');
        }
    }, []);

    const onKeyDown = useCallback((e) => {
        if (e.key === 'Enter') onSave();
    }, [onSave]);

    const dirty = draft.trim() !== saved;
    const empty = saved === '';

    return html`
        <section class="dev-section">
            <h2>Tooling gh account</h2>
            <p class="dev-section-hint">
                Username (e.g. <code>KMRH47</code>) used by qol-cicd's <code>activate.sh</code>
                to scope <code>GH_TOKEN</code> to qol-tools repos. Leave blank to disable.
            </p>
            <div class="link-input-row">
                <input type="text" id="tooling-gh-account-input"
                    placeholder="KMRH47"
                    value=${draft}
                    onInput=${(e) => setDraft(e.target.value)}
                    onKeyDown=${onKeyDown}
                    disabled=${status === 'saving'} />
                <button class="btn btn-sm btn-primary"
                    onClick=${onSave}
                    disabled=${status === 'saving' || !dirty}>
                    Save
                </button>
                <button class="btn btn-sm btn-ghost"
                    onClick=${onClear}
                    disabled=${status === 'saving' || empty}>
                    Clear
                </button>
            </div>
            ${error && html`<p class="error-msg">${error}</p>`}
            ${status === 'saved' && !error && html`<p class="last-action">Saved</p>`}
        </section>
    `;
}
