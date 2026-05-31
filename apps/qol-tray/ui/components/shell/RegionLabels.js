import { html } from '../../lib/html.js';
import { useEffect, useLayoutEffect, useRef } from 'preact/hooks';
import { createDebug } from '../../lib/debug.js';
import { resolveViewLabel } from '../../app/views.js';
import { HIDE_BELOW_SCREEN_W } from '../../lib/world-geometry.js';
import { ScrambleText } from '../../lib/components/ScrambleText.js';

const log = createDebug('qol:world');

function PlainText({ text }) {
    return html`<span>${text}</span>`;
}

const ANIMATIONS = {
    scramble: ScrambleText,
};

function writePositions(labelRefs, entries) {
    const layerEl = document.querySelector('.world-region-label-layer');
    if (!layerEl) return;
    const slotById = new Map();
    for (const slot of document.querySelectorAll('.world-view-slot')) {
        slotById.set(slot.dataset.viewId, slot);
    }
    const layerRect = layerEl.getBoundingClientRect();
    const work = [];
    for (const entry of entries) {
        const el = labelRefs.current.get(entry.id);
        if (!el) continue;
        const slot = slotById.get(entry.id);
        if (!slot) { el.style.display = 'none'; continue; }
        work.push({ el, rect: slot.getBoundingClientRect(), width: entry.width });
    }
    for (const { el, rect, width } of work) {
        if (rect.width < HIDE_BELOW_SCREEN_W) {
            el.style.display = 'none';
            continue;
        }
        const scale = width > 0 ? rect.width / width : 1;
        el.style.display = '';
        el.style.left = `${rect.left + rect.width / 2 - layerRect.left}px`;
        el.style.top = `${rect.top - layerRect.top}px`;
        el.style.transform = `translate(-50%, -50%) scale(${scale})`;
        el.style.maxWidth = `${rect.width}px`;
    }
}

export function RegionLabels({ registry, cameraLayer, navigation, diveDepth, camera }) {
    const layer = cameraLayer ?? 0;
    const pages = navigation?.getConfinedPages?.() || [];
    const ascending = layer < 0 && (diveDepth ?? 0) === 0;
    const allEntries = ascending ? [] : registry.getEntriesForLayer(layer);
    const entries = pages.length > 0
        ? allEntries.filter(e => pages.includes(e.id))
        : allEntries;

    const labelRefs = useRef(new Map());
    const entriesRef = useRef(entries);
    entriesRef.current = entries;
    const rafRef = useRef(0);

    const schedule = () => {
        if (rafRef.current) return;
        rafRef.current = requestAnimationFrame(() => {
            rafRef.current = 0;
            writePositions(labelRefs, entriesRef.current);
        });
    };

    useEffect(() => {
        if (!camera?.subscribe) return undefined;
        schedule();
        const unsub = camera.subscribe(schedule);
        return () => {
            unsub();
            if (rafRef.current) cancelAnimationFrame(rafRef.current);
            rafRef.current = 0;
        };
    }, [camera]);

    const entriesKey = entries.map(e => e.id).join('|');
    useLayoutEffect(() => {
        schedule();
    }, [entriesKey, camera]);

    const prevRef = useRef(null);
    const key = `${layer}:${entries.length}`;
    if (prevRef.current !== key) {
        prevRef.current = key;
        log('regionLabels: layer', layer,
            `entries=${entries.length}/${allEntries.length}`,
            pages.length > 0 ? 'confined' : 'all',
            entries.map(e => resolveViewLabel(e).text).join(', '));
    }

    const setLabelRef = (id) => (el) => {
        if (el) {
            labelRefs.current.set(id, el);
        } else {
            labelRefs.current.delete(id);
        }
    };

    return html`
        <div class="world-region-label-layer">
            ${entries.map(e => {
                const { text, animation } = resolveViewLabel(e);
                const Renderer = ANIMATIONS[animation] || PlainText;
                return html`
                    <div key=${e.id} ref=${setLabelRef(e.id)} class="world-region-label">
                        <${Renderer} text=${text} />
                    </div>
                `;
            })}
        </div>
    `;
}
