import { html } from '../lib/html.js';
import { useState, useCallback } from 'preact/hooks';
import { ModChips, ModChipsStatic } from './ModChips.js';

export function ScrollRules({ rules, onChange }) {
    const [fromMods, setFromMods] = useState([]);
    const [toMods, setToMods] = useState([]);
    const [global, setGlobal] = useState(false);

    const addRule = useCallback(() => {
        onChange([...rules, { from_mods: fromMods, to_mods: toMods, global }]);
        setFromMods([]); setToMods([]); setGlobal(false);
    }, [fromMods, toMods, global, rules, onChange]);

    const removeRule = useCallback((index) => {
        onChange(rules.filter((_, i) => i !== index));
    }, [rules, onChange]);

    return html`
        <section class="card">
            <h2>Scroll Rules</h2>
            <p>Remap modifier keys for scroll events (e.g. Ctrl+scroll to Cmd+scroll for zoom).</p>
            <div class="rules-list">
                ${rules.length === 0 && html`<div class="empty-state">No scroll rules defined.</div>`}
                ${rules.map((rule, i) => html`
                    <div class="rule-row">
                        <div class="rule-side"><${ModChipsStatic} mods=${rule.from_mods} /> <span class="key-label">scroll</span></div>
                        <span class="arrow">\u2192</span>
                        <div class="rule-side"><${ModChipsStatic} mods=${rule.to_mods} /> <span class="key-label">scroll</span></div>
                        ${rule.global && html`<span class="global-badge">global</span>`}
                        <button class="btn-remove" onClick=${() => removeRule(i)}>\u00d7</button>
                    </div>
                `)}
            </div>
            <div class="add-rule-row">
                <div class="rule-side"><${ModChips} selected=${fromMods} onChange=${setFromMods} /></div>
                <span class="arrow">\u2192</span>
                <div class="rule-side"><${ModChips} selected=${toMods} onChange=${setToMods} /></div>
                <label class="global-toggle">
                    <input type="checkbox" checked=${global} onChange=${(e) => setGlobal(e.target.checked)} />
                    <span>global</span>
                </label>
                <button class="btn-add" onClick=${addRule}>+ Add</button>
            </div>
        </section>
    `;
}
