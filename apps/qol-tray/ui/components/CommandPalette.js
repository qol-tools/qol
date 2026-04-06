import { html } from '../lib/html.js';
import { useRef, useState, useEffect, useMemo, useCallback } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { getCommands } from '../palette/registry.js';
import init, { fuzzy_match as wasmFuzzyMatch } from '../wasm/qol_wasm.js';
import { Surface } from './Surface.js';

function filterCommands(commands, query, useWasm) {
    if (!query) return commands;
    if (useWasm) {
        return commands
            .map(c => ({ cmd: c, match: wasmFuzzyMatch(query, c.label) }))
            .filter(({ match }) => match !== null)
            .sort((a, b) => a.match.score - b.match.score)
            .map(({ cmd }) => cmd);
    }
    const q = query.toLowerCase();
    return commands.filter(c => c.label.toLowerCase().includes(q));
}

export function CommandPalette() {
    const { active, query, mode, actionQuery, activeViewId, activate, deactivate, setQuery } = usePaletteContext();
    const inputRef = useRef(null);
    const [selectedIndex, setSelectedIndex] = useState(0);
    const [wasmLoaded, setWasmLoaded] = useState(false);

    useEffect(() => {
        let cancelled = false;
        init()
            .then(() => {
                if (cancelled) return;
                setWasmLoaded(true);
            })
            .catch(console.error);
        return () => {
            cancelled = true;
        };
    }, []);

    const commands = useMemo(() => {
        if (!active || mode !== 'action') return [];
        const raw = getCommands(activeViewId);
        const filtered = filterCommands(raw, actionQuery, wasmLoaded);
        console.log('[palette] raw:', raw.length, '| query:', JSON.stringify(actionQuery), '| filtered:', filtered.length, '| wasm:', wasmLoaded);
        return filtered;
    }, [active, mode, activeViewId, actionQuery, wasmLoaded]);

    useEffect(() => {
        setSelectedIndex(0);
    }, [commands]);

    useEffect(() => {
        if (active) inputRef.current?.focus();
    }, [active]);

    const handleBlur = useCallback(() => {
        setTimeout(() => {
            if (!inputRef.current?.matches(':focus')) deactivate();
        }, 0);
    }, [deactivate]);

    const handleInput = useCallback((e) => {
        const val = e.target.value;
        setQuery(val.startsWith('>') ? val.slice(1) : val);
    }, [setQuery]);

    const executeCommand = useCallback((cmd) => {
        deactivate();
        cmd.run();
    }, [deactivate]);

    const handleKeyDown = useCallback((e) => {
        if (e.key === 'Escape') {
            e.preventDefault();
            e.stopPropagation();
            deactivate();
            return;
        }
        if (mode !== 'action' || commands.length === 0) return;
        if (e.key === 'ArrowDown') {
            e.preventDefault();
            setSelectedIndex(i => i + 1 >= commands.length ? 0 : i + 1);
            return;
        }
        if (e.key === 'ArrowUp') {
            e.preventDefault();
            setSelectedIndex(i => i - 1 < 0 ? commands.length - 1 : i - 1);
            return;
        }
        if (e.key === 'Enter') {
            e.preventDefault();
            e.stopPropagation();
            const cmd = commands[selectedIndex];
            if (cmd) executeCommand(cmd);
        }
    }, [mode, commands, selectedIndex, deactivate, executeCommand]);

    const handleClick = useCallback(() => {
        if (!active) activate();
    }, [active, activate]);

    const clampedIndex = Math.min(selectedIndex, Math.max(0, commands.length - 1));

    if (!active) {
        return html`<div class="search-bar palette-hint" onClick=${handleClick}>
            <span class="palette-hint-text">Ctrl+E to search & run actions...</span>
        </div>`;
    }

    return html`<div class="command-palette">
        <div class="search-bar">
            <input ref=${inputRef} class="search-input" type="text"
                value=${query} onInput=${handleInput} onKeyDown=${handleKeyDown} onBlur=${handleBlur}
                placeholder=${mode === 'action' ? 'Type a command...' : 'Search...'} />
        </div>
        ${mode === 'action' && commands.length > 0 && html`
            <ul class="palette-dropdown">
                ${commands.map((cmd, i) => html`
                    <${Surface} as="li" key=${cmd.id}
                        className="palette-item ${i === clampedIndex ? 'selected' : ''}"
                        selected=${i === clampedIndex}
                        data-selected-surface-priority="10"
                        data-scroll-follow-mode="nearest"
                        onMouseDown=${() => executeCommand(cmd)}>
                        <span class="palette-item-label" data-selected-text="">${cmd.label}</span>
                    <//>
                `)}
            </ul>
        `}
    </div>`;
}
