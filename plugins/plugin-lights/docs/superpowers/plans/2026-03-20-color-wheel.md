# Color Wheel Controls Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace step-button light controls with a canvas hue+saturation disc and gradient sliders for live Zigbee color control.

**Architecture:** Backend adds two config-driven daemon actions (`set-brightness-main`, `set-colortemp-main`) following the existing `set-color-main` pattern. Frontend replaces `buildControls()` with a self-contained `hue-wheel.js` module that renders a canvas disc and DOM sliders with throttled pointer events.

**Tech Stack:** Rust (backend daemon), Vanilla JS + Canvas API (frontend), Zigbee ZCL

**Spec:** `docs/superpowers/specs/2026-03-20-color-wheel-design.md`

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `src/config/model.rs` | Modify | Add `live_brightness` and `live_mirek` fields to `PluginConfig` |
| `src/runtime/actions.rs` | Modify | Add `SET_BRIGHTNESS_MAIN` and `SET_COLORTEMP_MAIN` constants |
| `src/daemon/state.rs` | Modify | Add `apply_live_brightness()` and `apply_live_colortemp()` handlers |
| `plugin.toml` | Modify | Register two new actions |
| `ui/components/hue-wheel.js` | Create | Self-contained hue disc + sliders module |
| `ui/style.css` | Modify | Remove `.color-picker`, add wheel/slider styles |
| `ui/app.js` | Modify | Replace `buildControls()`, remove `buildControlRow()`, add render-survival |

---

### Task 1: Config schema — add live_brightness and live_mirek

**Files:**
- Modify: `src/config/model.rs:9-31`

- [ ] **Step 1: Add fields to PluginConfig struct**

In `src/config/model.rs`, add two fields after `live_color_hex` (line 16):

```rust
#[serde(default = "default_brightness")]
pub live_brightness: u8,
#[serde(default = "default_mirek")]
pub live_mirek: u16,
```

Add corresponding lines in the `Default` impl (after `live_color_hex: default_color(),` on line 27):

```rust
live_brightness: default_brightness(),
live_mirek: default_mirek(),
```

Add the default functions after the existing `default_color()` (after line 159):

```rust
fn default_brightness() -> u8 {
    100
}

fn default_mirek() -> u16 {
    300
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check` from plugin-lights root.
Expected: success, no errors.

- [ ] **Step 3: Commit**

```bash
git add src/config/model.rs
git commit -m "feat: add live_brightness and live_mirek to config schema"
```

---

### Task 2: Action constants and daemon handlers

**Files:**
- Modify: `src/runtime/actions.rs:1-67`
- Modify: `src/daemon/state.rs:61-103,112-131`

- [ ] **Step 1: Add action constants**

In `src/runtime/actions.rs`, add after `SET_COLOR_MAIN` (line 18):

```rust
pub const SET_BRIGHTNESS_MAIN: &str = "set-brightness-main";
pub const SET_COLORTEMP_MAIN: &str = "set-colortemp-main";
```

Add to `RUN_ACTIONS` array (after `SET_COLOR_MAIN` on line 37):

```rust
    SET_BRIGHTNESS_MAIN,
    SET_COLORTEMP_MAIN,
```

Add to `ALL_ACTIONS` array (after `SET_COLOR_MAIN` on line 58):

```rust
    SET_BRIGHTNESS_MAIN,
    SET_COLORTEMP_MAIN,
```

- [ ] **Step 2: Add handler match arms**

In `src/daemon/state.rs`, add after the `SET_COLOR_MAIN` handler (after line 91):

```rust
if action == actions::SET_BRIGHTNESS_MAIN {
    return self.apply_live_brightness();
}
if action == actions::SET_COLORTEMP_MAIN {
    return self.apply_live_colortemp();
}
```

- [ ] **Step 3: Implement apply_live_brightness()**

In `src/daemon/state.rs`, add after `apply_live_color()` (after line 125):

```rust
fn apply_live_brightness(&mut self) -> DaemonOutcome {
    let config = match store::load() {
        Ok(c) => c,
        Err(e) => return DaemonOutcome::Error(e.to_string()),
    };
    self.current_brightness = config.live_brightness;
    self.apply(LightCommand::SetBrightness {
        level: config.live_brightness,
    })
}
```

- [ ] **Step 4: Implement apply_live_colortemp()**

Add right after `apply_live_brightness()`:

```rust
fn apply_live_colortemp(&mut self) -> DaemonOutcome {
    let config = match store::load() {
        Ok(c) => c,
        Err(e) => return DaemonOutcome::Error(e.to_string()),
    };
    self.current_mirek = config.live_mirek;
    self.apply(LightCommand::SetColorTemperature {
        mirek: config.live_mirek,
    })
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check`
Expected: success.

- [ ] **Step 6: Commit**

```bash
git add src/runtime/actions.rs src/daemon/state.rs
git commit -m "feat: add set-brightness-main and set-colortemp-main actions"
```

---

### Task 3: Register new actions in plugin.toml

**Files:**
- Modify: `plugin.toml:11-29`

- [ ] **Step 1: Add action registrations**

In `plugin.toml`, add after `set-color-main` (line 29):

```toml
set-brightness-main = ["set-brightness-main"]
set-colortemp-main = ["set-colortemp-main"]
```

- [ ] **Step 2: Commit**

```bash
git add plugin.toml
git commit -m "feat: register set-brightness-main and set-colortemp-main actions"
```

---

### Task 4: Create hue-wheel.js — disc + sliders + interaction

**Files:**
- Create: `ui/components/hue-wheel.js`

This is the core UI module. It handles all rendering, pointer events, throttling, and color math. The full module:

- [ ] **Step 1: Create components directory and write hue-wheel.js**

```javascript
const DISC_SIZE = 280;
const DISC_THUMB_R = 9;
const SLIDER_THUMB_SIZE = 28;
const MIREK_MIN = 153;
const MIREK_MAX = 500;

export function createHueWheel(container, {
    onColorChange,
    onBrightnessChange,
    onColorTempChange,
    initialState = {},
    throttleMs = 150,
} = {}) {
    let hue = initialState.hue ?? 0;
    let sat = initialState.saturation ?? 1;
    let brightness = initialState.brightness ?? 100;
    let mirek = initialState.mirek ?? 300;
    let tracking = null;
    let throttleTimer = null;
    let lastFireTime = 0;

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
    const ctSlider = buildSlider('colortemp', 'Color Temp');

    wrapper.append(canvas, bSlider.el, ctSlider.el);
    container.appendChild(wrapper);

    drawDisc();
    syncBrightnessSlider();
    syncColorTempSlider();

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
        if (type === 'colortemp') {
            track.style.background =
                'linear-gradient(to right, #9dbfff, #fff5e6, #ffcc7a, #ffaa44)';
        }

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

        return { el, track, thumb, value };
    }

    function syncBrightnessSlider() {
        const hex = hueSatToHex(hue, sat);
        bSlider.track.style.background =
            `linear-gradient(to right, #0a0a0a, #${hex})`;
        bSlider.thumb.style.left = `${brightness}%`;
        bSlider.value.textContent = `${brightness}%`;
    }

    function syncColorTempSlider() {
        const pct = (mirek - MIREK_MIN) / (MIREK_MAX - MIREK_MIN) * 100;
        ctSlider.thumb.style.left = `${pct}%`;
        ctSlider.value.textContent = `${Math.round(1000000 / mirek)}K`;
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
        throttledFire(() => onColorChange?.(hueSatToHex(hue, sat)));
    }

    function handleSliderPointer(e, type, track) {
        const rect = track.getBoundingClientRect();
        const pct = Math.max(0, Math.min(1,
            (e.clientX - rect.left) / rect.width));

        if (type === 'brightness') {
            brightness = Math.round(pct * 100);
            syncBrightnessSlider();
            throttledFire(() => onBrightnessChange?.(brightness));
        } else {
            mirek = Math.round(MIREK_MIN + pct * (MIREK_MAX - MIREK_MIN));
            syncColorTempSlider();
            throttledFire(() => onColorTempChange?.(mirek));
        }
    }

    function finalize(type) {
        tracking = null;
        clearTimeout(throttleTimer);
        throttleTimer = null;
        lastFireTime = 0;

        if (type === 'disc') onColorChange?.(hueSatToHex(hue, sat));
        else if (type === 'brightness') onBrightnessChange?.(brightness);
        else if (type === 'colortemp') onColorTempChange?.(mirek);
    }

    function throttledFire(fn) {
        const now = Date.now();
        if (now - lastFireTime >= throttleMs) {
            lastFireTime = now;
            fn();
            return;
        }
        clearTimeout(throttleTimer);
        throttleTimer = setTimeout(() => {
            lastFireTime = Date.now();
            fn();
        }, throttleMs - (now - lastFireTime));
    }

    return {
        setHue(h) { hue = h; drawDisc(); syncBrightnessSlider(); },
        setSaturation(s) { sat = s; drawDisc(); syncBrightnessSlider(); },
        setBrightness(b) { brightness = b; syncBrightnessSlider(); },
        setMirek(m) { mirek = m; syncColorTempSlider(); },
        destroy() {
            clearTimeout(throttleTimer);
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
```

- [ ] **Step 2: Commit**

```bash
git add ui/components/hue-wheel.js
git commit -m "feat: add hue-wheel.js canvas disc and slider module"
```

---

### Task 5: CSS styles for wheel and sliders

**Files:**
- Modify: `ui/style.css:92-100,307-347`

- [ ] **Step 1: Remove .color-picker rule**

Delete lines 92-100 (the `.color-picker` block).

- [ ] **Step 2: Remove step-button control styles**

Delete `.control-row` (lines 313-317), `.control-label` (lines 319-323), `.toggle-btn` (lines 326-333), `.btn-group` (lines 335-342), and `.controls-disabled` (lines 344-347) — these were only used by the step buttons and are replaced by wheel styles.

- [ ] **Step 3: Add wheel and slider styles**

Add after the `.controls-hint` rule:

```css
.hue-wheel-container {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 28px;
}

.hue-disc {
    border-radius: 50%;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4), 0 0 0 1px rgba(255, 255, 255, 0.05);
    cursor: crosshair;
    touch-action: none;
}

.hue-slider {
    width: 100%;
    max-width: 380px;
}

.hue-slider-header {
    display: flex;
    justify-content: space-between;
    margin-bottom: 8px;
    align-items: baseline;
}

.hue-slider-label {
    font-size: 12px;
    color: #888;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-weight: 500;
}

.hue-slider-value {
    font-size: 12px;
    color: #666;
    font-variant-numeric: tabular-nums;
}

.hue-slider-track {
    height: 36px;
    border-radius: 18px;
    position: relative;
    box-shadow: inset 0 2px 4px rgba(0, 0, 0, 0.5), 0 0 0 1px rgba(255, 255, 255, 0.05);
    cursor: pointer;
    touch-action: none;
}

.hue-slider-thumb {
    position: absolute;
    top: 50%;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: white;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.4);
    transform: translate(-50%, -50%);
    pointer-events: none;
}

.toggle-btn {
    width: 100%;
    padding: 16px;
    font-size: 16px;
    font-weight: 600;
    border-radius: 14px;
    letter-spacing: 0.03em;
    background: linear-gradient(135deg, #e94560, #c73550);
    box-shadow: 0 2px 12px rgba(233, 69, 96, 0.3);
    margin-bottom: 28px;
}
```

- [ ] **Step 4: Commit**

```bash
git add ui/style.css
git commit -m "feat: add hue wheel and slider styles, remove step-button styles"
```

---

### Task 6: Wire hue-wheel into app.js

**Files:**
- Modify: `ui/app.js:1-671`

- [ ] **Step 1: Add import and state variables**

At the top of `ui/app.js`, add as line 1:

```javascript
import { createHueWheel, hexToHueSat } from './components/hue-wheel.js';
```

After the existing state variables (after line 16), add:

```javascript
let hueWheelInstance = null;
let controlsSection = null;
let controlsDeviceId = null;
```

- [ ] **Step 2: Replace buildControls()**

Replace the entire `buildControls()` function (lines 341-407) with:

```javascript
function buildControls() {
    const active = !!config.main_target_id;

    if (controlsSection && controlsDeviceId === config.main_target_id) {
        return controlsSection;
    }

    if (hueWheelInstance) {
        hueWheelInstance.destroy();
        hueWheelInstance = null;
    }

    controlsDeviceId = config.main_target_id;

    const section = document.createElement('div');
    section.className = 'section';

    const title = document.createElement('div');
    title.className = 'section-title';
    title.textContent = 'Controls';
    section.appendChild(title);

    if (!active) {
        const hint = document.createElement('div');
        hint.className = 'controls-hint';
        hint.textContent = 'Set a main device to enable controls';
        section.appendChild(hint);
        controlsSection = section;
        return section;
    }

    const grid = document.createElement('div');
    grid.className = 'controls-grid';

    const toggleBtn = document.createElement('button');
    toggleBtn.className = 'btn btn-accent toggle-btn';
    toggleBtn.textContent = 'Toggle';
    toggleBtn.addEventListener('click', function () { sendAction('toggle-main', this); });
    grid.appendChild(toggleBtn);

    const wheelContainer = document.createElement('div');
    grid.appendChild(wheelContainer);

    const { hue, saturation } = hexToHueSat(config.live_color_hex || 'ffffff');

    hueWheelInstance = createHueWheel(wheelContainer, {
        onColorChange(hex) {
            config.live_color_hex = hex;
            silentSaveConfig(config);
            sendAction('set-color-main');
        },
        onBrightnessChange(percent) {
            config.live_brightness = percent;
            silentSaveConfig(config);
            sendAction('set-brightness-main');
        },
        onColorTempChange(mirek) {
            config.live_mirek = mirek;
            silentSaveConfig(config);
            sendAction('set-colortemp-main');
        },
        initialState: {
            hue,
            saturation,
            brightness: config.live_brightness ?? 100,
            mirek: config.live_mirek ?? 300,
        },
    });

    section.appendChild(grid);
    controlsSection = section;
    return section;
}
```

- [ ] **Step 3: Add silentSaveConfig()**

Add after the existing `saveConfig()` function. This saves config without triggering `render()`, preventing `replaceChildren()` from detaching the canvas mid-drag and breaking pointer capture:

```javascript
async function silentSaveConfig(updated) {
    try {
        const res = await fetch(CONFIG_URL, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(updated),
        });
        if (res.ok) config = updated;
    } catch (_) {}
}
```

- [ ] **Step 4: Remove buildControlRow()**

Delete the `buildControlRow()` function (lines 409-432 in the original file). It is no longer called.

- [ ] **Step 5: Commit**

```bash
git add ui/app.js
git commit -m "feat: wire hue wheel into controls, remove step-button UI"
```

---

### Task 7: Manual integration verification

This task has no code changes. Verify the full flow works end-to-end.

- [ ] **Step 1: Build the binary**

Run: `cargo build --release` from plugin-lights root.
Expected: compiles without errors.

- [ ] **Step 2: Start the daemon and open UI**

Kill any running plugin-lights daemon. Start fresh. Open the settings UI in a browser at `http://127.0.0.1:42700/plugins/plugin-lights/`.

- [ ] **Step 3: Verify disc renders**

Expected: 280px hue+saturation disc visible below the toggle button. White center, pure hues at edge. Thumb indicator visible.

- [ ] **Step 4: Verify disc interaction**

Click and drag on the disc. Expected: thumb follows pointer, `set-color-main` action fires (check daemon logs). Color on physical light changes.

- [ ] **Step 5: Verify brightness slider**

Drag the brightness slider. Expected: thumb moves, value label updates (0-100%), `set-brightness-main` action fires, light brightness changes.

- [ ] **Step 6: Verify color temp slider**

Drag the color temp slider. Expected: thumb moves, value shows Kelvin, `set-colortemp-main` action fires, light temperature changes.

- [ ] **Step 7: Verify render survival**

Wait 5+ seconds (the `refreshData` timer). Expected: the wheel, sliders, and thumb positions persist without resetting. The controls section is NOT rebuilt.

- [ ] **Step 8: Verify throttling**

Drag quickly across the disc. Expected: commands fire at ~150ms intervals (visible in daemon logs), not on every pixel of movement. Final position always sent on release.
