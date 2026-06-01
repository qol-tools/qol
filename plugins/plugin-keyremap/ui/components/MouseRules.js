import { html } from '../lib/html.js';
import { useState, useCallback } from 'preact/hooks';
import { ModChips, ModChipsStatic } from './ModChips.js';

export function MouseRules({ rules, onChange }) {
    const [fromMods, setFromMods] = useState([]);
    const [button, setButton] = useState('left');
    const [toMods, setToMods] = useState([]);
    const [global, setGlobal] = useState(false);

    const addRule = useCallback(() => {
        onChange([...rules, { from_mods: fromMods, button, to_mods: toMods, global }]);
        setFromMods([]); setToMods([]); setGlobal(false);
    }, [fromMods, button, toMods, global, rules, onChange]);

    const removeRule = useCallback((index) => {
        onChange(rules.filter((_, i) => i !== index));
    }, [rules, onChange]);

    return html`
        <section class="card">
            <h2>Mouse Rules</h2>
            <p>Remap modifier keys for mouse clicks (e.g. Ctrl+click to Cmd+click).</p>
            <div class="rules-list">
                ${rules.length === 0 && html`<div class="empty-state">No mouse rules defined.</div>`}
                ${rules.map((rule, i) => html`
                    <div class="rule-row">
                        <div class="rule-side"><${ModChipsStatic} mods=${rule.from_mods} /> <span class="key-label">${rule.button} click</span></div>
                        <span class="arrow">\u2192</span>
                        <div class="rule-side"><${ModChipsStatic} mods=${rule.to_mods} /></div>
                        ${rule.global && html`<span class="global-badge">global</span>`}
                        <button class="btn-remove" onClick=${() => removeRule(i)}>\u00d7</button>
                    </div>
                `)}
            </div>
            <div class="add-rule-row">
                <div class="rule-side">
                    <${ModChips} selected=${fromMods} onChange=${setFromMods} />
                    <select class="key-input" value=${button} onChange=${(e) => setButton(e.target.value)}>
                        <option value="left">left</option>
                        <option value="right">right</option>
                    </select>
                </div>
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
