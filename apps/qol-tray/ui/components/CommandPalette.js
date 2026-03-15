import { html } from '../lib/html.js';
import { useRef, useState, useEffect, useMemo, useCallback } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { getCommands } from '../palette/registry.js';
import init, { fuzzy_match as wasmFuzzyMatch } from '../wasm/qol_wasm.js';

function filterCommands(commands, query) {
    if (!query) return commands;
    return commands
        .map(c => ({ cmd: c, match: wasmFuzzyMatch(query, c.label) }))
        .filter(({ match }) => match !== null)
        .sort((a, b) => a.match.score - b.match.score)
        .map(({ cmd }) => cmd);
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

    const commands = useMemo(
        () => mode === 'action' && wasmLoaded ? filterCommands(getCommands(activeViewId), actionQuery) : [],
        [mode, activeViewId, actionQuery, wasmLoaded]
    );

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
        setQuery(e.target.value);
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
                    <li key=${cmd.id} class="palette-item ${i === clampedIndex ? 'selected' : ''}" data-selected-surface="" data-selected=${i === clampedIndex ? 'true' : 'false'} data-selected-surface-priority="10" data-scroll-follow-mode="nearest"
                        onMouseDown=${() => executeCommand(cmd)}>
                        <span class="palette-item-label" data-selected-text="">${cmd.label}</span>
                    </li>
                `)}
            </ul>
        `}
    </div>`;
}
