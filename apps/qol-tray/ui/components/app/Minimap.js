import { html } from '../../lib/html.js';
import { useEffect, useRef, useState } from 'preact/hooks';
import { VIEW_LABELS } from './views.js';

const NEIGHBORHOOD = 3;
const PAGE_PAD = 0.3;
const LABEL_FONT = 9;
const ACTIVE_LABEL_FONT = 10;

export function Minimap({ camera, registry, viewportRef }) {
    const canvasRef = useRef(null);
    const [, bump] = useState(0);

    useEffect(() => {
        return camera.subscribe(() => bump(t => t + 1));
    }, [camera]);

    useEffect(() => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const ctx = canvas.getContext('2d');
        const dpr = window.devicePixelRatio || 1;
        const cw = canvas.clientWidth;
        const ch = canvas.clientHeight;
        canvas.width = cw * dpr;
        canvas.height = ch * dpr;
        ctx.scale(dpr, dpr);

        const currentLayer = camera.layer;
        const vp = viewportRef?.current;
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;
        const z = camera.zoom || 1;

        const entries = registry.getEntriesForLayer(currentLayer);
        if (entries.length === 0) return;

        const camCenterX = camera.x + vpW / (2 * z);
        const camCenterY = camera.y + vpH / (2 * z);
        const sorted = entries
            .map(e => ({ e, dist: Math.abs(e.x + e.width / 2 - camCenterX) }))
            .sort((a, b) => a.dist - b.dist);
        const neighbors = sorted.slice(0, NEIGHBORHOOD).map(s => s.e);

        const nb = neighborhoodBounds(neighbors, PAGE_PAD);
        const scale = Math.min(cw / nb.width, ch / nb.height);
        const offsetX = (cw - nb.width * scale) / 2;
        const offsetY = (ch - nb.height * scale) / 2;

        const activeId = registry.activeViewId(camera.x, camera.y, vpW, vpH, z);

        ctx.clearRect(0, 0, cw, ch);

        drawPositionDots(ctx, entries, neighbors, nb, scale, offsetX, offsetY);

        for (const e of neighbors) {
            const rx = (e.x - nb.x) * scale + offsetX;
            const ry = (e.y - nb.y) * scale + offsetY;
            const rw = e.width * scale;
            const rh = e.height * scale;
            const active = e.id === activeId;

            ctx.fillStyle = active ? 'rgba(255,255,255,0.18)' : 'rgba(255,255,255,0.06)';
            roundRect(ctx, rx, ry, rw, rh, 3);
            ctx.fill();
            ctx.strokeStyle = active ? 'rgba(255,255,255,0.6)' : 'rgba(255,255,255,0.18)';
            ctx.lineWidth = active ? 1.5 : 0.5;
            ctx.stroke();

            const label = VIEW_LABELS[e.id] || e.id;
            const fontSize = active ? ACTIVE_LABEL_FONT : LABEL_FONT;
            ctx.fillStyle = active ? 'rgba(255,255,255,0.9)' : 'rgba(255,255,255,0.45)';
            ctx.font = `${active ? 'bold ' : ''}${fontSize}px -apple-system, sans-serif`;
            ctx.textAlign = 'center';
            ctx.textBaseline = 'middle';
            ctx.fillText(label, rx + rw / 2, ry + rh / 2, rw - 6);
        }

        if (vp) {
            const worldVpW = vpW / z;
            const worldVpH = vpH / z;
            const vpX = (camera.x - nb.x) * scale + offsetX;
            const vpY = (camera.y - nb.y) * scale + offsetY;
            const vpWs = worldVpW * scale;
            const vpHs = worldVpH * scale;
            ctx.strokeStyle = 'rgba(255,255,255,0.5)';
            ctx.lineWidth = 1;
            ctx.setLineDash([3, 3]);
            ctx.strokeRect(vpX, vpY, vpWs, vpHs);
            ctx.setLineDash([]);
        }

        const layerLabel = currentLayer === 0 ? 'L0' : `L${currentLayer}`;
        ctx.fillStyle = 'rgba(255,255,255,0.4)';
        ctx.font = 'bold 8px -apple-system, sans-serif';
        ctx.textAlign = 'right';
        ctx.textBaseline = 'bottom';
        ctx.fillText(layerLabel, cw - 4, ch - 3);
    });

    const onClick = (e) => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const rect = canvas.getBoundingClientRect();
        const currentLayer = camera.layer;
        const entries = registry.getEntriesForLayer(currentLayer);
        if (entries.length === 0) return;

        const vp = viewportRef?.current;
        const z = camera.zoom || 1;
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;
        const camCenterX = camera.x + vpW / (2 * z);
        const sorted = entries
            .map(en => ({ e: en, dist: Math.abs(en.x + en.width / 2 - camCenterX) }))
            .sort((a, b) => a.dist - b.dist);
        const neighbors = sorted.slice(0, NEIGHBORHOOD).map(s => s.e);
        const nb = neighborhoodBounds(neighbors, PAGE_PAD);
        const scale = Math.min(canvas.clientWidth / nb.width, canvas.clientHeight / nb.height);
        const offsetX = (canvas.clientWidth - nb.width * scale) / 2;
        const offsetY = (canvas.clientHeight - nb.height * scale) / 2;
        const wx = nb.x + (e.clientX - rect.left - offsetX) / scale - vpW / (2 * z);
        const wy = nb.y + (e.clientY - rect.top - offsetY) / scale - vpH / (2 * z);
        camera.panSmooth(wx, wy, 300);
    };

    return html`
        <div class="world-minimap" onClick=${onClick}>
            <canvas ref=${canvasRef} style="width:100%;height:100%"></canvas>
        </div>
    `;
}

function neighborhoodBounds(entries, padFraction) {
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (const e of entries) {
        minX = Math.min(minX, e.x);
        minY = Math.min(minY, e.y);
        maxX = Math.max(maxX, e.x + e.width);
        maxY = Math.max(maxY, e.y + e.height);
    }
    const w = maxX - minX;
    const h = maxY - minY;
    const px = w * padFraction;
    const py = h * padFraction;
    return { x: minX - px, y: minY - py, width: w + px * 2, height: h + py * 2 };
}

function drawPositionDots(ctx, allEntries, neighbors, nb, scale, offsetX, offsetY) {
    const neighborIds = new Set(neighbors.map(e => e.id));
    for (const e of allEntries) {
        if (neighborIds.has(e.id)) continue;
        const cx = (e.x + e.width / 2 - nb.x) * scale + offsetX;
        const cy = (e.y + e.height / 2 - nb.y) * scale + offsetY;
        ctx.fillStyle = 'rgba(255,255,255,0.15)';
        ctx.beginPath();
        ctx.arc(cx, cy, 2, 0, Math.PI * 2);
        ctx.fill();
    }
}

function roundRect(ctx, x, y, w, h, r) {
    ctx.beginPath();
    ctx.moveTo(x + r, y);
    ctx.lineTo(x + w - r, y);
    ctx.quadraticCurveTo(x + w, y, x + w, y + r);
    ctx.lineTo(x + w, y + h - r);
    ctx.quadraticCurveTo(x + w, y + h, x + w - r, y + h);
    ctx.lineTo(x + r, y + h);
    ctx.quadraticCurveTo(x, y + h, x, y + h - r);
    ctx.lineTo(x, y + r);
    ctx.quadraticCurveTo(x, y, x + r, y);
    ctx.closePath();
}
