import { html } from '../lib/html.js';
import { useState, useCallback } from 'preact/hooks';
import { ModChips, ModChipsStatic } from './ModChips.js';

function BatchForm({ onAdd }) {
    const [fromMods, setFromMods] = useState([]);
    const [toMods, setToMods] = useState([]);
    const [keys, setKeys] = useState('');
    const [global, setGlobal] = useState(false);

    const submit = useCallback(() => {
        const parsed = keys.split(',').map(k => k.trim().toLowerCase()).filter(Boolean);
        if (parsed.length === 0) return;
        onAdd({ from_mods: fromMods, to_mods: toMods, keys: parsed, global });
        setFromMods([]); setToMods([]); setKeys(''); setGlobal(false);
    }, [fromMods, toMods, keys, global, onAdd]);

    return html`
        <div class="add-rule-row">
            <div class="rule-side"><${ModChips} selected=${fromMods} onChange=${setFromMods} /></div>
            <span class="arrow">\u2192</span>
            <div class="rule-side"><${ModChips} selected=${toMods} onChange=${setToMods} /></div>
            <input type="text" class="text-input keys-input" placeholder="keys (comma-separated, e.g. c, v, x)"
                value=${keys} onInput=${(e) => setKeys(e.target.value)}
                onKeyDown=${(e) => e.key === 'Enter' && (e.preventDefault(), submit())} />
            <label class="global-toggle">
                <input type="checkbox" checked=${global} onChange=${(e) => setGlobal(e.target.checked)} />
                <span>global</span>
            </label>
            <button class="btn-add" onClick=${submit}>+ Add</button>
        </div>
    `;
}

function SingleForm({ onAdd }) {
    const [fromMods, setFromMods] = useState([]);
    const [fromKey, setFromKey] = useState('');
    const [toMods, setToMods] = useState([]);
    const [toKey, setToKey] = useState('');
    const [global, setGlobal] = useState(false);

    const submit = useCallback(() => {
        const fk = fromKey.trim().toLowerCase();
        const tk = toKey.trim().toLowerCase();
        if (!fk || !tk) return;
        onAdd({ from_mods: fromMods, from_key: fk, to_mods: toMods, to_key: tk, global });
        setFromMods([]); setFromKey(''); setToMods([]); setToKey(''); setGlobal(false);
    }, [fromMods, fromKey, toMods, toKey, global, onAdd]);

    return html`
        <div class="add-rule-row">
            <div class="rule-side">
                <${ModChips} selected=${fromMods} onChange=${setFromMods} />
                <input type="text" class="key-input" placeholder="from key"
                    value=${fromKey} onInput=${(e) => setFromKey(e.target.value)}
                    onKeyDown=${(e) => e.key === 'Enter' && (e.preventDefault(), submit())} />
            </div>
            <span class="arrow">\u2192</span>
            <div class="rule-side">
                <${ModChips} selected=${toMods} onChange=${setToMods} />
                <input type="text" class="key-input" placeholder="to key"
                    value=${toKey} onInput=${(e) => setToKey(e.target.value)}
                    onKeyDown=${(e) => e.key === 'Enter' && (e.preventDefault(), submit())} />
            </div>
            <label class="global-toggle">
                <input type="checkbox" checked=${global} onChange=${(e) => setGlobal(e.target.checked)} />
                <span>global</span>
            </label>
            <button class="btn-add" onClick=${submit}>+ Add</button>
        </div>
    `;
}

export function KeyRules({ rules, onChange, onClearWarnings }) {
    const [tab, setTab] = useState('batch');

    const addRule = useCallback((rule) => {
        onClearWarnings();
        onChange([...rules, rule]);
    }, [rules, onChange, onClearWarnings]);

    const removeRule = useCallback((index) => {
        onClearWarnings();
        onChange(rules.filter((_, i) => i !== index));
    }, [rules, onChange, onClearWarnings]);

    return html`
        <section class="card">
            <h2>Key Rules</h2>
            <p>Remap keyboard shortcuts. First matching rule wins.</p>
            <div class="rules-list">
                ${rules.length === 0 && html`<div class="empty-state">No key rules defined.</div>`}
                ${rules.map((rule, i) => html`
                    <div class="rule-row ${rule.keys ? 'batch-rule' : ''}">
                        <div class="rule-side">
                            <${ModChipsStatic} mods=${rule.from_mods} />
                            ${rule.keys
                                ? html`${rule.keys.map(k => html`<span class="key-chip">${k}</span>`)}`
                                : html`<span class="key-label">${rule.from_key}</span>`
                            }
                        </div>
                        <span class="arrow">\u2192</span>
                        <div class="rule-side">
                            <${ModChipsStatic} mods=${rule.to_mods} />
                            ${rule.keys
                                ? html`<span class="key-label-hint">same key</span>`
                                : html`<span class="key-label">${rule.to_key}</span>`
                            }
                        </div>
                        ${rule.global && html`<span class="global-badge">global</span>`}
                        <button class="btn-remove" onClick=${() => removeRule(i)}>\u00d7</button>
                    </div>
                `)}
            </div>
            <div class="add-rule-tabs">
                <button class="tab-btn ${tab === 'batch' ? 'active' : ''}" onClick=${() => setTab('batch')}>Batch (same key)</button>
                <button class="tab-btn ${tab === 'single' ? 'active' : ''}" onClick=${() => setTab('single')}>Single (key changes)</button>
            </div>
            ${tab === 'batch' && html`<${BatchForm} onAdd=${addRule} />`}
            ${tab === 'single' && html`<${SingleForm} onAdd=${addRule} />`}
        </section>
    `;
}
