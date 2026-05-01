import { html } from '../../lib/html.js';
import { useEffect, useLayoutEffect, useRef } from 'preact/hooks';
import { createDebug } from '../../lib/debug.js';
import { resolveViewLabel } from '../../app/views.js';
import { regionLabelPosition } from '../../lib/world-geometry.js';
import { ScrambleText } from '../../lib/components/ScrambleText.js';

const log = createDebug('qol:world');

function PlainText({ text }) {
    return html`<span>${text}</span>`;
}

const ANIMATIONS = {
    scramble: ScrambleText,
};

function writePositions(labelRefs, entries, cam) {
    for (const entry of entries) {
        const el = labelRefs.current.get(entry.id);
        if (!el) continue;
        const pos = regionLabelPosition(entry, cam);
        if (pos.hidden) {
            el.style.display = 'none';
            continue;
        }
        el.style.display = '';
        el.style.left = `${pos.left}px`;
        el.style.top = `${pos.top}px`;
        el.style.maxWidth = `${pos.maxWidth}px`;
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

    useEffect(() => {
        if (!camera?.subscribe) return undefined;
        writePositions(labelRefs, entriesRef.current, {
            x: camera.x, y: camera.y, zoom: camera.zoom,
        });
        const unsub = camera.subscribe((cam) => {
            writePositions(labelRefs, entriesRef.current, cam);
        });
        return unsub;
    }, [camera]);

    const entriesKey = entries.map(e => e.id).join('|');
    useLayoutEffect(() => {
        if (!camera) return;
        writePositions(labelRefs, entries, {
            x: camera.x,
            y: camera.y,
            zoom: camera.zoom,
        });
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
