import { html } from '../../lib/html.js';
import { useEffect, useLayoutEffect, useRef, useState } from 'preact/hooks';
import {
    computePeripheralSlots,
    computeSiblingCoverage,
    handleSlotClick,
    pickCenteredEntry,
    shouldHidePeripheralSide,
} from '../../lib/peripheral-geometry.js';
import { getWorldSettings } from '../../lib/world-settings.js';
import { useOverlayHide } from '../../lib/hooks/useIdleHide.js';

const DEFAULT_NEIGHBORS = 1;
const ANIM_DURATION_MS = 240;
const ANIM_EASING = 'cubic-bezier(0.2, 0.8, 0.25, 1)';
const PARALLAX_PX = 28;
const SCALE_FOR_DISTANCE = { 1: 0.8, 2: 0.6, 3: 0.4, 4: 0.28 };
const BASE_OPACITY_FOR_DISTANCE = { 1: 0.75, 2: 0.45, 3: 0.25, 4: 0.15 };
const SLOT_CSS_W = { min: 300, vw: 0.40, max: 600 };
const SLOT_CSS_H = { min: 260, vh: 0.50, max: 660 };

function computeSlotBoxSize(vpW, vpH) {
    return {
        w: Math.max(SLOT_CSS_W.min, Math.min(SLOT_CSS_W.max, vpW * SLOT_CSS_W.vw)),
        h: Math.max(SLOT_CSS_H.min, Math.min(SLOT_CSS_H.max, vpH * SLOT_CSS_H.vh)),
    };
}

function computeMiniScale(entry, slotBox) {
    if (!entry || entry.width <= 0 || entry.height <= 0) return 1;
    return Math.min(slotBox.w / entry.width, slotBox.h / entry.height);
}

export function PeripheralPreview({ camera, navigation, registry, renderPage }) {
    const [, bump] = useState(0);

    useEffect(() => {
        if (!navigation?.subscribeAnchor) return undefined;
        return navigation.subscribeAnchor(() => bump((t) => t + 1));
    }, [navigation]);

    useEffect(() => {
        if (!camera?.subscribe) return undefined;
        return camera.subscribe(() => bump((t) => t + 1));
    }, [camera]);

    const slotRefs = useRef(new Map());
    const prevIdxRef = useRef(null);
    const animationsRef = useRef(new Map());

    useEffect(() => () => {
        for (const anim of animationsRef.current.values()) anim.cancel();
        animationsRef.current.clear();
    }, []);

    useEffect(() => {
        const onResize = () => bump((t) => t + 1);
        window.addEventListener('resize', onResize);
        return () => window.removeEventListener('resize', onResize);
    }, []);

    const confinedPages = navigation?.getConfinedPages?.() || [];
    const viewportSize = { w: window.innerWidth, h: window.innerHeight };
    const entries = confinedPages
        .map((id) => registry?.getEntry?.(id))
        .filter(Boolean);
    const centered = camera ? pickCenteredEntry(entries, camera, viewportSize) : null;
    const activePageId = centered?.id || navigation?.getCurrentAnchor?.()?.pageId || null;
    const idx = activePageId ? confinedPages.indexOf(activePageId) : -1;

    useLayoutEffect(() => {
        if (idx < 0) { prevIdxRef.current = null; return; }
        const prevIdx = prevIdxRef.current;
        prevIdxRef.current = idx;
        if (prevIdx == null || prevIdx === idx) return;
        const step = idx - prevIdx;
        if (!step) return;
        if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
        const direction = step > 0 ? 1 : -1;
        for (const [key, el] of slotRefs.current) {
            if (!el || !el.isConnected) continue;
            const prior = animationsRef.current.get(key);
            const fromStyle = prior ? captureRunningState(el) : null;
            if (prior) prior.cancel();
            const anim = animateSlot(el, key, direction, fromStyle);
            if (!anim) continue;
            animationsRef.current.set(key, anim);
            anim.finished.then(
                () => { if (animationsRef.current.get(key) === anim) animationsRef.current.delete(key); },
                () => {},
            );
        }
    }, [idx, activePageId]);

    const slots = computePeripheralSlots(activePageId, confinedPages, DEFAULT_NEIGHBORS);
    if (slots.length === 0) return null;

    const slotBox = computeSlotBoxSize(viewportSize.w, viewportSize.h);
    const activeEntry = activePageId ? registry?.getEntry?.(activePageId) : null;

    return html`
        <div class="peripheral-preview" aria-hidden="true">
            ${slots.map((slot) => {
                const key = `${slot.side}-${slot.distance}`;
                const entry = slot.id ? registry?.getEntry?.(slot.id) : null;
                const coverage = entry && camera
                    ? computeSiblingCoverage(entry, camera, viewportSize)
                    : 0;
                const hideSide = activeEntry && camera
                    ? shouldHidePeripheralSide({ side: slot.side, activeEntry, camera, viewport: viewportSize })
                    : false;
                if (hideSide) return null;
                const base = BASE_OPACITY_FOR_DISTANCE[slot.distance] ?? 0.08;
                const coverageOpacity = Math.max(0, 1 - coverage) * base;
                const miniScale = computeMiniScale(entry, slotBox);
                return html`<${PeripheralSlot}
                    key=${key}
                    slotKey=${key}
                    slot=${slot}
                    entry=${entry}
                    renderPage=${renderPage}
                    miniScale=${miniScale}
                    coverageOpacity=${coverageOpacity}
                    camera=${camera}
                    navigation=${navigation}
                    slotRefs=${slotRefs}
                />`;
            })}
        </div>
    `;
}

function PeripheralSlot({ slotKey, slot, entry, renderPage, miniScale, coverageOpacity, camera, navigation, slotRefs }) {
    const ref = useRef(null);
    useOverlayHide({ targetRef: ref, camera, navigation });
    const isEmpty = !slot.id;
    const contentStyle = entry
        ? `width:${entry.width}px;height:${entry.height}px;--peripheral-mini-scale:${miniScale};`
        : '';
    const slotStyle = isEmpty ? '' : `--coverage-opacity:${coverageOpacity};`;
    const setRef = (el) => {
        ref.current = el;
        if (el) slotRefs.current.set(slotKey, el);
        else slotRefs.current.delete(slotKey);
    };
    return html`
        <button
            type="button"
            class=${`peripheral-slot peripheral-slot-${slot.side}${isEmpty ? ' peripheral-slot-empty' : ''}`}
            data-distance=${slot.distance}
            tabindex="-1"
            style=${slotStyle}
            disabled=${isEmpty}
            onClick=${isEmpty ? undefined : () => handleSlotClick(slot, navigation, getWorldSettings().defaultZoom)}
            ref=${setRef}
        >
            ${slot.id && renderPage
                ? html`<div class="peripheral-mini">
                    <div class="peripheral-mini-content" style=${contentStyle}>${renderPage(slot.id)}</div>
                </div>`
                : html`<div class="peripheral-edge"></div>`}
        </button>
    `;
}

function captureRunningState(el) {
    const cs = getComputedStyle(el);
    return { transform: cs.transform, opacity: cs.opacity };
}

function animateSlot(el, key, direction, fromStyle) {
    const [side, distStr] = key.split('-');
    const distance = Number(distStr);
    const restScale = SCALE_FOR_DISTANCE[distance] ?? 0.2;
    const restOpacity = BASE_OPACITY_FOR_DISTANCE[distance] ?? 0.05;

    const sideSign = side === 'next' ? 1 : -1;
    const movingTowardThisSide = sideSign === direction;
    const fromScale = movingTowardThisSide
        ? (SCALE_FOR_DISTANCE[distance + 1] ?? Math.max(0.18, restScale - 0.18))
        : (SCALE_FOR_DISTANCE[Math.max(1, distance - 1)] ?? Math.min(0.95, restScale + 0.18));
    const fromOpacity = movingTowardThisSide
        ? (BASE_OPACITY_FOR_DISTANCE[distance + 1] ?? Math.max(0.04, restOpacity * 0.5))
        : (BASE_OPACITY_FOR_DISTANCE[Math.max(1, distance - 1)] ?? Math.min(1, restOpacity * 2));

    const parallaxX = direction * PARALLAX_PX * (1 / distance);

    const fromTransform = fromStyle?.transform && fromStyle.transform !== 'none'
        ? fromStyle.transform
        : `translate(${parallaxX}px, -50%) scale(${fromScale})`;
    const fromOpacityResolved = fromStyle?.opacity != null
        ? Number(fromStyle.opacity)
        : fromOpacity;
    const toTransform = `translate(0px, -50%) scale(${restScale})`;

    return el.animate(
        [
            { transform: fromTransform, opacity: fromOpacityResolved },
            { transform: toTransform, opacity: restOpacity },
        ],
        {
            duration: ANIM_DURATION_MS,
            easing: ANIM_EASING,
            fill: 'none',
        },
    );
}
