import { html } from '../../lib/html.js';
import { useEffect, useRef, useState } from 'preact/hooks';
import { VIEW_LABELS } from './views.js';

const MIN_PAGE_W = 18;
const MIN_PAGE_H = 14;
const LABEL_FONT_SIZE = 8;

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
        const bounds = registry.worldBounds(currentLayer);
        if (bounds.width === 0) return;
        const scale = Math.min(cw / bounds.width, ch / bounds.height);

        const vp = viewportRef?.current;
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;
        const z = camera.zoom || 1;
        const activeId = registry.activeViewId(camera.x, camera.y, vpW, vpH, z);

        ctx.clearRect(0, 0, cw, ch);

        for (const e of registry.getEntriesForLayer(currentLayer)) {
            const cx = (e.x + e.width / 2 - bounds.x) * scale;
            const cy = (e.y + e.height / 2 - bounds.y) * scale;
            const rw = Math.max(MIN_PAGE_W, e.width * scale);
            const rh = Math.max(MIN_PAGE_H, e.height * scale);
            const rx = cx - rw / 2;
            const ry = cy - rh / 2;
            const active = e.id === activeId;

            ctx.fillStyle = active ? 'rgba(255,255,255,0.15)' : 'rgba(255,255,255,0.06)';
            roundRect(ctx, rx, ry, rw, rh, 2);
            ctx.fill();
            ctx.strokeStyle = active ? 'rgba(255,255,255,0.5)' : 'rgba(255,255,255,0.15)';
            ctx.lineWidth = active ? 1 : 0.5;
            ctx.stroke();

            const label = VIEW_LABELS[e.id] || e.id;
            ctx.fillStyle = active ? 'rgba(255,255,255,0.85)' : 'rgba(255,255,255,0.4)';
            ctx.font = `${active ? 'bold ' : ''}${LABEL_FONT_SIZE}px -apple-system, sans-serif`;
            ctx.textAlign = 'center';
            ctx.textBaseline = 'middle';
            ctx.fillText(label, cx, cy, rw - 4);
        }

        if (vp) {
            const worldVpW = vpW / z;
            const worldVpH = vpH / z;
            const vpX = (camera.x - bounds.x) * scale;
            const vpY = (camera.y - bounds.y) * scale;
            const vpWs = worldVpW * scale;
            const vpHs = worldVpH * scale;
            ctx.strokeStyle = 'rgba(255,255,255,0.6)';
            ctx.lineWidth = 1.5;
            ctx.strokeRect(vpX, vpY, vpWs, vpHs);
        }

        const layerLabel = currentLayer === 0 ? 'L0' : `L${currentLayer}`;
        ctx.fillStyle = 'rgba(255,255,255,0.5)';
        ctx.font = 'bold 9px -apple-system, sans-serif';
        ctx.textAlign = 'right';
        ctx.textBaseline = 'bottom';
        ctx.fillText(layerLabel, cw - 4, ch - 3);
    });

    const onClick = (e) => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const rect = canvas.getBoundingClientRect();
        const currentLayer = camera.layer;
        const bounds = registry.worldBounds(currentLayer);
        if (bounds.width === 0) return;
        const scale = Math.min(canvas.clientWidth / bounds.width, canvas.clientHeight / bounds.height);
        const vp = viewportRef?.current;
        const z = camera.zoom || 1;
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;
        const wx = bounds.x + (e.clientX - rect.left) / scale - vpW / (2 * z);
        const wy = bounds.y + (e.clientY - rect.top) / scale - vpH / (2 * z);
        camera.panSmooth(wx, wy, 300);
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
