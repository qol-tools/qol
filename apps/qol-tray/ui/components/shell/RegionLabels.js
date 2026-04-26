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

// Data-driven renderer lookup — add a new label animation by adding an entry
// here and setting `animation: 'your-key'` on the matching VIEW_LABELS entry.
// Never branch on the animation string with if/else in this file.
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

    // Labels render in screen space (outside #world) so they don't scale with
    // the world transform. Per-frame positions are written imperatively inside
    // the camera subscribe callback so labels update in the SAME frame as
    // #world's transform (see world-camera.js::apply).
    const labelRefs = useRef(new Map());
    const entriesRef = useRef(entries);
    entriesRef.current = entries;

    useEffect(() => {
        if (!camera?.subscribe) return undefined;
        // Write once on subscribe using live camera getters — the parent's
        // mount-time `gotoAnchor` often settles BEFORE our subscription is
        // registered (parent useLayoutEffect runs after child ones), so we
        // can't rely on a future notify() to arrive. Without this sync, labels
        // stay frozen at their initial stale position until the next camera
        // interaction.
        writePositions(labelRefs, entriesRef.current, {
            x: camera.x, y: camera.y, zoom: camera.zoom,
        });
        const unsub = camera.subscribe((cam) => {
            writePositions(labelRefs, entriesRef.current, cam);
        });
        return unsub;
    }, [camera]);

    // When the set of visible entries changes (layer switch, dive, confinement
    // change), compute positions once immediately — the next camera tick might
    // not come if nothing else is moving.
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
