const DISC_SIZE = 280;
const DISC_THUMB_R = 9;

export function createHueWheel(container, {
    onRelease,
    wsUrl = 'ws://127.0.0.1:42710',
    initialState = {},
} = {}) {
    let hue = initialState.hue ?? 0;
    let sat = initialState.saturation ?? 1;
    let brightness = initialState.brightness ?? 100;
    let tracking = null;
    let ws = null;
    let wsReconnectTimer = null;

    const wrapper = document.createElement('div');
    wrapper.className = 'hue-wheel-container';
    wrapper.id = 'hue-wheel';

    const canvas = document.createElement('canvas');
    canvas.width = DISC_SIZE;
    canvas.height = DISC_SIZE;
    canvas.className = 'hue-disc';
    const ctx = canvas.getContext('2d');

    const offscreen = document.createElement('canvas');
    offscreen.width = DISC_SIZE;
    offscreen.height = DISC_SIZE;
    prerenderDisc();

    const bSlider = buildSlider('brightness', 'Brightness');

    wrapper.append(canvas, bSlider.el);
    container.appendChild(wrapper);

    drawDisc();
    syncBrightnessSlider();
    connectWs();

    function connectWs() {
        try {
            ws = new WebSocket(wsUrl);
            ws.onclose = () => scheduleReconnect();
            ws.onerror = () => {};
        } catch (_) {
            scheduleReconnect();
        }
    }

    function scheduleReconnect() {
        ws = null;
        clearTimeout(wsReconnectTimer);
        wsReconnectTimer = setTimeout(connectWs, 2000);
    }

    function wsSend(obj) {
        if (ws?.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify(obj));
        }
    }

    function prerenderDisc() {
        const offCtx = offscreen.getContext('2d');
        const w = DISC_SIZE;
        const center = w / 2;
        const radius = center - 1;
        const imageData = offCtx.createImageData(w, w);
        const d = imageData.data;

        for (let py = 0; py < w; py++) {
            for (let px = 0; px < w; px++) {
                const dx = px - center;
                const dy = py - center;
                const dist = Math.sqrt(dx * dx + dy * dy);
                if (dist > radius) continue;

                const angle = Math.atan2(dy, dx);
                const h = ((angle * 180 / Math.PI) + 360) % 360;
                const s = dist / radius;
                const [hr, hg, hb] = hueComponents(h);

                const i = (py * w + px) * 4;
                d[i]     = Math.round((1 - s + s * hr) * 255);
                d[i + 1] = Math.round((1 - s + s * hg) * 255);
                d[i + 2] = Math.round((1 - s + s * hb) * 255);
                d[i + 3] = 255;
            }
        }

        offCtx.putImageData(imageData, 0, 0);
    }

    function drawDisc() {
        ctx.clearRect(0, 0, DISC_SIZE, DISC_SIZE);
        ctx.drawImage(offscreen, 0, 0);

        const center = DISC_SIZE / 2;
        const radius = center - 1;
        const angle = hue * Math.PI / 180;
        const dist = sat * radius;
        const tx = center + dist * Math.cos(angle);
        const ty = center + dist * Math.sin(angle);

        ctx.save();
        ctx.beginPath();
        ctx.arc(tx, ty, DISC_THUMB_R, 0, Math.PI * 2);
        ctx.fillStyle = '#' + hueSatToHex(hue, sat);
        ctx.fill();
        ctx.lineWidth = 3;
        ctx.strokeStyle = 'white';
        ctx.shadowColor = 'rgba(0,0,0,0.5)';
        ctx.shadowBlur = 6;
        ctx.stroke();
        ctx.restore();
    }

    function buildSlider(type, labelText) {
        const el = document.createElement('div');
        el.className = 'hue-slider';

        const header = document.createElement('div');
        header.className = 'hue-slider-header';

        const label = document.createElement('span');
        label.className = 'hue-slider-label';
        label.textContent = labelText;

        const value = document.createElement('span');
        value.className = 'hue-slider-value';

        header.append(label, value);

        const track = document.createElement('div');
        track.className = 'hue-slider-track';

        const thumb = document.createElement('div');
        thumb.className = 'hue-slider-thumb';

        track.appendChild(thumb);
        el.append(header, track);

        track.addEventListener('pointerdown', (e) => {
            tracking = type;
            track.setPointerCapture(e.pointerId);
            handleSliderPointer(e, type, track);
        });

        track.addEventListener('pointermove', (e) => {
            if (tracking === type) handleSliderPointer(e, type, track);
        });

        track.addEventListener('pointerup', () => {
            if (tracking === type) finalize(type);
        });

        track.addEventListener('pointercancel', () => {
            if (tracking === type) finalize(type);
        });

        return { el, track, thumb, value };
    }

    function syncBrightnessSlider() {
        const hex = hueSatToHex(hue, sat);
        bSlider.track.style.background =
            `linear-gradient(to right, #0a0a0a, #${hex})`;
        bSlider.thumb.style.left = `${brightness}%`;
        bSlider.value.textContent = `${brightness}%`;
    }

    canvas.addEventListener('pointerdown', (e) => {
        const rect = canvas.getBoundingClientRect();
        const x = e.clientX - rect.left;
        const y = e.clientY - rect.top;
        const center = DISC_SIZE / 2;
        const dist = Math.sqrt((x - center) ** 2 + (y - center) ** 2);

        if (dist > center - 1) return;

        tracking = 'disc';
        canvas.setPointerCapture(e.pointerId);
        handleDiscPointer(x, y);
    });

    canvas.addEventListener('pointermove', (e) => {
        if (tracking !== 'disc') return;
        const rect = canvas.getBoundingClientRect();
        handleDiscPointer(e.clientX - rect.left, e.clientY - rect.top);
    });

    canvas.addEventListener('pointerup', () => {
        if (tracking === 'disc') finalize('disc');
    });

    canvas.addEventListener('pointercancel', () => {
        if (tracking === 'disc') finalize('disc');
    });

    function handleDiscPointer(x, y) {
        const center = DISC_SIZE / 2;
        const radius = center - 1;
        const dx = x - center;
        const dy = y - center;
        const dist = Math.min(Math.sqrt(dx * dx + dy * dy), radius);

        hue = ((Math.atan2(dy, dx) * 180 / Math.PI) + 360) % 360;
        sat = dist / radius;

        drawDisc();
        syncBrightnessSlider();
        wsSend({ type: 'color', hex: hueSatToHex(hue, sat) });
    }

    function handleSliderPointer(e, type, track) {
        const rect = track.getBoundingClientRect();
        const pct = Math.max(0, Math.min(1,
            (e.clientX - rect.left) / rect.width));

        brightness = Math.max(1, Math.round(pct * 100));
        syncBrightnessSlider();
        wsSend({ type: 'brightness', level: brightness, hex: hueSatToHex(hue, sat) });
    }

    function finalize(type) {
        tracking = null;

        const hex = hueSatToHex(hue, sat);
        if (type === 'disc') {
            wsSend({ type: 'color', hex });
        } else if (type === 'brightness') {
            wsSend({ type: 'brightness', level: brightness, hex });
        }

        onRelease?.({ hex, brightness, hue, saturation: sat });
    }

    return {
        setHue(h) { hue = h; drawDisc(); syncBrightnessSlider(); },
        setSaturation(s) { sat = s; drawDisc(); syncBrightnessSlider(); },
        setBrightness(b) { brightness = b; syncBrightnessSlider(); },
        destroy() {
            clearTimeout(wsReconnectTimer);
            if (ws) ws.close();
            wrapper.remove();
        },
    };
}

function hueComponents(h) {
    const x = 1 - Math.abs(((h / 60) % 2) - 1);
    if (h < 60)  return [1, x, 0];
    if (h < 120) return [x, 1, 0];
    if (h < 180) return [0, 1, x];
    if (h < 240) return [0, x, 1];
    if (h < 300) return [x, 0, 1];
    return [1, 0, x];
}

function hueSatToHex(h, s) {
    const [hr, hg, hb] = hueComponents(h);
    const r = Math.round((1 - s + s * hr) * 255);
    const g = Math.round((1 - s + s * hg) * 255);
    const b = Math.round((1 - s + s * hb) * 255);
    return [r, g, b].map(c => c.toString(16).padStart(2, '0')).join('');
}

export function hexToHueSat(hex) {
    const r = parseInt(hex.substring(0, 2), 16) / 255;
    const g = parseInt(hex.substring(2, 4), 16) / 255;
    const b = parseInt(hex.substring(4, 6), 16) / 255;

    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    const delta = max - min;

    let h = 0;
    if (delta > 0) {
        if (max === r) h = 60 * (((g - b) / delta + 6) % 6);
        else if (max === g) h = 60 * ((b - r) / delta + 2);
        else h = 60 * ((r - g) / delta + 4);
    }

    const s = max === 0 ? 0 : delta / max;
    return { hue: h, saturation: s };
}
