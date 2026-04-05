import { html } from '../../lib/html.js';
import { useEffect, useRef, useState } from 'preact/hooks';
import { VIEW_LABELS } from './views.js';

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

        const bounds = registry.worldBounds();
        if (bounds.width === 0) return;
        const scale = Math.min(cw / bounds.width, ch / bounds.height);

        const vp = viewportRef?.current;
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;
        const activeId = registry.activeViewId(camera.x, camera.y, vpW, vpH);

        ctx.clearRect(0, 0, cw, ch);

        for (const e of registry.getAllEntries()) {
            const rx = (e.x - bounds.x) * scale;
            const ry = (e.y - bounds.y) * scale;
            const rw = e.width * scale;
            const rh = e.height * scale;
            const active = e.id === activeId;

            ctx.fillStyle = active ? 'rgba(255,255,255,0.12)' : 'rgba(255,255,255,0.04)';
            ctx.fillRect(rx, ry, rw, rh);
            ctx.strokeStyle = active ? 'rgba(255,255,255,0.4)' : 'rgba(255,255,255,0.12)';
            ctx.lineWidth = active ? 1 : 0.5;
            ctx.strokeRect(rx, ry, rw, rh);

            const label = VIEW_LABELS[e.id] || e.id;
            ctx.fillStyle = active ? 'rgba(255,255,255,0.7)' : 'rgba(255,255,255,0.3)';
            ctx.font = `${active ? 'bold ' : ''}7px -apple-system, sans-serif`;
            ctx.textAlign = 'center';
            ctx.textBaseline = 'middle';
            ctx.fillText(label, rx + rw / 2, ry + rh / 2, rw - 2);
        }

        if (vp) {
            const vpX = (camera.x - bounds.x) * scale;
            const vpY = (camera.y - bounds.y) * scale;
            const vpWs = vpW * scale;
            const vpHs = vpH * scale;
            ctx.strokeStyle = 'rgba(255,255,255,0.6)';
            ctx.lineWidth = 1.5;
            ctx.strokeRect(vpX, vpY, vpWs, vpHs);
        }
    });

    const onClick = (e) => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const rect = canvas.getBoundingClientRect();
        const bounds = registry.worldBounds();
        if (bounds.width === 0) return;
        const scale = Math.min(canvas.clientWidth / bounds.width, canvas.clientHeight / bounds.height);
        const vp = viewportRef?.current;
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;
        const wx = bounds.x + (e.clientX - rect.left) / scale - vpW / 2;
        const wy = bounds.y + (e.clientY - rect.top) / scale - vpH / 2;
        camera.panSmooth(wx, wy, 300);
    };

    return html`
        <div class="world-minimap" onClick=${onClick}>
            <canvas ref=${canvasRef} style="width:100%;height:100%"></canvas>
        </div>
    `;
}
