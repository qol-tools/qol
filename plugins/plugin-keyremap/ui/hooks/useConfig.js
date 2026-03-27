import { useState, useEffect, useCallback } from 'preact/hooks';

const PLUGIN_ID = window.location.pathname.split('/')[2];
const CONFIG_URL = `/api/plugins/${PLUGIN_ID}/config`;
const VALID_MODS = new Set(['ctrl', 'shift', 'alt', 'cmd', 'ralt', 'altgr']);

const DEFAULT_CONFIG = {
    enabled: true,
    excluded_apps: [],
    char_swaps: [],
    char_rules: [],
    key_rules: [],
    mouse_rules: [],
    scroll_rules: [],
};

function normalizeMods(mods) {
    if (!Array.isArray(mods)) return [];
    return mods.filter(m => VALID_MODS.has(m));
}

function normalizeKeyRule(r) {
    if (r.keys) {
        return {
            from_mods: normalizeMods(r.from_mods),
            to_mods: normalizeMods(r.to_mods),
            keys: r.keys.filter(k => typeof k === 'string' && k.length > 0),
            global: !!r.global,
        };
    }
    return {
        from_mods: normalizeMods(r.from_mods),
        from_key: String(r.from_key),
        to_mods: normalizeMods(r.to_mods),
        to_key: String(r.to_key),
        global: !!r.global,
    };
}

function normalizeConfig(raw) {
    return {
        enabled: typeof raw?.enabled === 'boolean' ? raw.enabled : true,
        excluded_apps: Array.isArray(raw?.excluded_apps)
            ? raw.excluded_apps.filter(a => typeof a === 'string' && a.length > 0)
            : [],
        char_swaps: Array.isArray(raw?.char_swaps)
            ? raw.char_swaps.filter(s => Array.isArray(s) && s.length === 2 && s[0] && s[1])
                .map(([a, b]) => [String(a), String(b)])
            : [],
        char_rules: Array.isArray(raw?.char_rules)
            ? raw.char_rules.filter(r => r?.from_key && r?.to_char).map(r => ({
                from_mods: normalizeMods(r.from_mods),
                from_key: String(r.from_key),
                to_char: String(r.to_char),
                global: !!r.global,
            }))
            : [],
        key_rules: Array.isArray(raw?.key_rules)
            ? raw.key_rules
                .filter(r => (r?.keys && r.keys.length > 0) || (r?.from_key && r?.to_key))
                .map(normalizeKeyRule)
            : [],
        mouse_rules: Array.isArray(raw?.mouse_rules)
            ? raw.mouse_rules.filter(r => r?.button).map(r => ({
                from_mods: normalizeMods(r.from_mods),
                button: String(r.button),
                to_mods: normalizeMods(r.to_mods),
                global: !!r.global,
            }))
            : [],
        scroll_rules: Array.isArray(raw?.scroll_rules)
            ? raw.scroll_rules.map(r => ({
                from_mods: normalizeMods(r?.from_mods),
                to_mods: normalizeMods(r?.to_mods),
                global: !!r.global,
            }))
            : [],
    };
}

function modsEqual(a, b) {
    if (a.length !== b.length) return false;
    const sa = [...a].sort();
    const sb = [...b].sort();
    return sa.every((m, i) => m === sb[i]);
}

export function validateKeyRules(rules) {
    const warnings = [];
    const batchRules = rules.filter(r => r.keys);

    for (let i = 0; i < batchRules.length; i++) {
        for (let j = i + 1; j < batchRules.length; j++) {
            const a = batchRules[i], b = batchRules[j];
            if (!modsEqual(a.from_mods, b.from_mods)) continue;

            const overlap = a.keys.filter(k => b.keys.includes(k));
            if (overlap.length > 0) {
                warnings.push(`Keys [${overlap.join(', ')}] appear in two [${a.from_mods.join('+')}] rules with different targets — only the first rule will fire`);
            }

            if (modsEqual(a.to_mods, b.to_mods)) {
                warnings.push(`Two batch rules [${a.from_mods.join('+')}] → [${a.to_mods.join('+')}] could be merged into one`);
            }
        }
    }

    const singleRules = rules.filter(r => r.from_key);
    for (const single of singleRules) {
        for (const batch of batchRules) {
            if (modsEqual(single.from_mods, batch.from_mods) && batch.keys.includes(single.from_key)) {
                warnings.push(`Single rule [${single.from_mods.join('+')}]+${single.from_key} is shadowed by an earlier batch rule`);
            }
        }
    }

    return warnings;
}

export { normalizeMods };

export function useConfig() {
    const [config, setConfig] = useState(DEFAULT_CONFIG);
    const [loading, setLoading] = useState(true);
    const [saveStatus, setSaveStatus] = useState('');
    const [saveError, setSaveError] = useState(false);
    const [warnings, setWarnings] = useState([]);
    const [pendingWarnings, setPendingWarnings] = useState(false);

    useEffect(() => {
        fetch(CONFIG_URL)
            .then(res => res.ok ? res.json() : DEFAULT_CONFIG)
            .then(raw => setConfig(normalizeConfig(raw)))
            .catch(() => {})
            .finally(() => setLoading(false));
    }, []);

    const save = useCallback(async () => {
        const w = validateKeyRules(config.key_rules);
        if (w.length > 0 && !pendingWarnings) {
            setWarnings(w);
            setPendingWarnings(true);
            return;
        }

        setPendingWarnings(false);
        setWarnings([]);
        setSaveStatus('Saving...');
        setSaveError(false);

        try {
            const res = await fetch(CONFIG_URL, {
                method: 'PUT',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(config, null, 2),
            });
            if (!res.ok) throw new Error(`Status ${res.status}`);
            setSaveStatus('Saved');
            setTimeout(() => setSaveStatus(''), 2000);
        } catch {
            setSaveStatus('Failed to save');
            setSaveError(true);
            setTimeout(() => { setSaveStatus(''); setSaveError(false); }, 3000);
        }
    }, [config, pendingWarnings]);

    const clearWarnings = useCallback(() => {
        setPendingWarnings(false);
        setWarnings([]);
    }, []);

    return { config, setConfig, loading, save, saveStatus, saveError, warnings, pendingWarnings, clearWarnings };
}
