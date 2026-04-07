import { html } from '../../lib/html.js';
import { useEffect, useRef, useState } from 'preact/hooks';
import { VIEW_LABELS } from './views.js';

const VISIBLE_COUNT = 5;
const PAGE_GAP = 6;
const PAGE_RADIUS = 3;
const PAD = 8;
const LABEL_FONT = 9;

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

        const sorted = [...entries].sort((a, b) => a.x - b.x);
        const activeId = registry.activeViewId(camera.x, camera.y, vpW, vpH, z);
        const activeIdx = sorted.findIndex(e => e.id === activeId);

        const half = Math.floor(VISIBLE_COUNT / 2);
        let startIdx = Math.max(0, activeIdx - half);
        if (startIdx + VISIBLE_COUNT > sorted.length) startIdx = Math.max(0, sorted.length - VISIBLE_COUNT);
        const visible = sorted.slice(startIdx, startIdx + VISIBLE_COUNT);
        const count = visible.length;

        const totalGap = (count - 1) * PAGE_GAP;
        const availW = cw - PAD * 2 - totalGap;
        const availH = ch - PAD * 2;
        const pageW = availW / count;
        const pageH = availH;

        ctx.clearRect(0, 0, cw, ch);

        for (let i = 0; i < count; i++) {
            const e = visible[i];
            const rx = PAD + i * (pageW + PAGE_GAP);
            const ry = PAD;
            const active = e.id === activeId;

            ctx.fillStyle = active ? 'rgba(255,255,255,0.18)' : 'rgba(255,255,255,0.06)';
            roundRect(ctx, rx, ry, pageW, pageH, PAGE_RADIUS);
            ctx.fill();
            ctx.strokeStyle = active ? 'rgba(255,255,255,0.6)' : 'rgba(255,255,255,0.18)';
            ctx.lineWidth = active ? 1.5 : 0.5;
            ctx.stroke();

            const label = VIEW_LABELS[e.id] || e.id;
            ctx.fillStyle = active ? 'rgba(255,255,255,0.9)' : 'rgba(255,255,255,0.45)';
            ctx.font = `${active ? 'bold ' : ''}${LABEL_FONT}px -apple-system, sans-serif`;
            ctx.textAlign = 'center';
            ctx.textBaseline = 'middle';
            ctx.fillText(label, rx + pageW / 2, ry + pageH / 2, pageW - 6);
        }

        if (startIdx > 0) {
            ctx.fillStyle = 'rgba(255,255,255,0.25)';
            ctx.font = 'bold 10px -apple-system, sans-serif';
            ctx.textAlign = 'left';
            ctx.textBaseline = 'middle';
            ctx.fillText('‹', 2, ch / 2);
        }
        if (startIdx + VISIBLE_COUNT < sorted.length) {
            ctx.fillStyle = 'rgba(255,255,255,0.25)';
            ctx.font = 'bold 10px -apple-system, sans-serif';
            ctx.textAlign = 'right';
            ctx.textBaseline = 'middle';
            ctx.fillText('›', cw - 2, ch / 2);
        }

        const layerLabel = currentLayer === 0 ? 'L0' : `L${currentLayer}`;
        ctx.fillStyle = 'rgba(255,255,255,0.3)';
        ctx.font = '7px -apple-system, sans-serif';
        ctx.textAlign = 'right';
        ctx.textBaseline = 'bottom';
        ctx.fillText(layerLabel, cw - 4, ch - 2);
    });

    const onClick = (e) => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const rect = canvas.getBoundingClientRect();
        const clickX = e.clientX - rect.left;

        const currentLayer = camera.layer;
        const entries = registry.getEntriesForLayer(currentLayer);
        if (entries.length === 0) return;

        const vp = viewportRef?.current;
        const z = camera.zoom || 1;
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;

        const sorted = [...entries].sort((a, b) => a.x - b.x);
        const activeId = registry.activeViewId(camera.x, camera.y, vpW, vpH, z);
        const activeIdx = sorted.findIndex(en => en.id === activeId);

        const half = Math.floor(VISIBLE_COUNT / 2);
        let startIdx = Math.max(0, activeIdx - half);
        if (startIdx + VISIBLE_COUNT > sorted.length) startIdx = Math.max(0, sorted.length - VISIBLE_COUNT);
        const visible = sorted.slice(startIdx, startIdx + VISIBLE_COUNT);
        const count = visible.length;

        const totalGap = (count - 1) * PAGE_GAP;
        const pageW = (canvas.clientWidth - PAD * 2 - totalGap) / count;
        const slotW = pageW + PAGE_GAP;
        const idx = Math.floor((clickX - PAD) / slotW);
        const target = visible[Math.max(0, Math.min(idx, count - 1))];
        if (!target) return;

        const tx = target.x + target.width / 2 - vpW / (2 * z);
        const ty = target.y + target.height / 2 - vpH / (2 * z);
        camera.panTo(tx, ty);
    };

    return html`
        <div class="world-minimap" onClick=${onClick}>
            <canvas ref=${canvasRef} style="width:100%;height:100%"></canvas>
        </div>
    `;
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
