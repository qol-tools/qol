# Color Wheel Controls Redesign

## Overview

Replace the step-button controls (dimmer/brighter, warmer/cooler) and native `<input type="color">` in plugin-lights with a canvas-based hue+saturation disc and two gradient sliders. Live control — colors change on the device in real-time as the user drags.

## Controls Layout

The controls section becomes a vertical stack:

1. **Toggle button** — full-width, gradient accent with glow
2. **Hue+Saturation disc** (280px) — full color disc, centered
3. **Brightness slider** (36px tall, full-width) — black to current color
4. **Color temp slider** (36px tall, full-width) — cool blue to warm amber, displayed in Kelvin

All existing step buttons (dimmer, brighter, warmer, cooler) and the native `<input type="color">` are removed.

## Hue+Saturation Disc

### Rendering

- Canvas element, 280x280px
- Pixel-by-pixel rendering to ImageData:
  - Angle from center = hue (0-360)
  - Distance from center = saturation (0 at center, 1 at edge)
  - White center fading to pure hues at the circumference
- Pre-render once to an OffscreenCanvas on init
- On interaction, `drawImage()` the cached disc onto the visible canvas, then draw the thumb on top

### Thumb Indicator

- 18px circle with 3px white border and drop shadow
- Positioned at the current (hue, saturation) point on the disc
- Updates position on every pointermove (visual feedback is instant)

### Hit Testing

- Calculate distance from pointer to canvas center
- If distance <= radius, the click is on the disc
- Compute hue from angle, saturation from distance/radius

## Sliders

### Brightness Slider

- Range: 0-100%
- Track gradient: black (#0a0a0a) → current selected color (reflects hue+saturation from disc)
- 28px white circular thumb with drop shadow
- Label: "Brightness" with percentage value right-aligned

### Color Temperature Slider

- Range: 153-500 mirek (internally)
- Display: converted to Kelvin (1,000,000 / mirek, rounded)
- Track gradient: cool blue (#9dbfff) → neutral white (#fff5e6) → warm amber (#ffaa44)
- 28px white circular thumb with drop shadow
- Label: "Color Temp" with Kelvin value right-aligned

### Slider Interaction

- Pointer events on the track element
- pointerdown starts tracking, pointermove updates, pointerup finalizes
- Same throttle behavior as the disc

## Interaction Model

### Pointer Events

- `pointerdown` on disc or slider starts tracking
- `pointermove` while tracking updates the visual thumb position immediately
- Zigbee commands are throttled: fire at most every 150ms during drag
- `pointerup` ends tracking, always sends the final value regardless of throttle timer
- Click without drag works — tap a spot, get that value

### Throttling

- Default: 150ms between Zigbee commands during drag
- Configurable via `throttleMs` parameter
- Visual feedback (thumb movement) is never throttled — only the hardware command is gated
- On release, the final value is always sent

## Data Flow

### Disc → Device

1. User drags on disc
2. Compute hue (angle) and saturation (distance/radius)
3. Convert (hue, saturation) → RGB hex using linear interpolation: `R = (1 - sat + sat * hueR) * 255`, same for G and B. This produces white (255,255,255) at sat=0 and pure hue at sat=1, matching the disc rendering.
4. Update config `live_color_hex`
5. Send `set-color-main` action
6. Daemon receives action, reads config hex
7. `parse_color()` → `rgb_to_cie_xy()` → ZCL cluster 768 command

### Brightness Slider → Device

1. User drags brightness slider
2. Compute percentage from horizontal position
3. Write percentage to config `live_brightness`
4. Save config via PUT
5. Send new `set-brightness-main` action
6. Daemon reads `live_brightness` from config, maps to ZCL level (0-254) → cluster 8 command

### Color Temp Slider → Device

1. User drags color temp slider
2. Compute mirek from horizontal position (153-500 range)
3. Write mirek to config `live_mirek`
4. Save config via PUT
5. Send new `set-colortemp-main` action
6. Daemon reads `live_mirek` from config, sends mirek via ZCL cluster 768

## File Changes

### New File: `ui/components/hue-wheel.js`

Self-contained vanilla JS module exporting:

```
createHueWheel(container, { onColorChange, onBrightnessChange, onColorTempChange, initialState, throttleMs })
```

- `container`: DOM element to render into
- `onColorChange(hex)`: called with 6-digit hex string on disc interaction
- `onBrightnessChange(percent)`: called with 0-100 on brightness slider interaction
- `onColorTempChange(mirek)`: called with 153-500 on color temp slider interaction
- `initialState`: `{ hue, saturation, brightness, mirek }` for initial thumb positions
- `throttleMs`: throttle interval (default 150)

Returns: `{ setHue(h), setSaturation(s), setBrightness(b), setMirek(m), destroy() }`

Responsible for:
- Canvas rendering (disc + thumb)
- Slider rendering (tracks + thumbs)
- All pointer event handling
- Throttle logic
- Color space conversion (hue+saturation → RGB hex via linear interpolation)

### Modified: `ui/app.js`

- `buildControls()`: replace step buttons and color picker with a container div, call `createHueWheel()` with callbacks that wire into existing `sendAction()` and `saveConfig()` functions
- Remove: `buildControlRow()` function (no longer needed)
- The hue wheel must survive `render()` cycles. Strategy: `buildControls()` checks if a wheel container already exists in the DOM (by ID). If it does, return it as-is without recreating. The wheel only rebuilds on first render or when the active device changes. This avoids losing drag state, canvas cache, and pointer tracking from the 5-second `refreshData` timer and other `render()` triggers.

### Modified: `ui/style.css`

- Remove: `.color-picker` rule
- Add: `.hue-wheel-container` layout styles (centering, spacing)
- Add: `.hue-slider-track`, `.hue-slider-thumb` styles
- Keep: `.controls-grid`, `.controls-disabled`, `.controls-hint`, `.toggle-btn` (still used)

## Styling

All styling follows the existing dark theme:
- Background: `linear-gradient(135deg, #1a1a2e, #16213e, #0f0f1a)`
- Card surfaces: `rgba(22, 33, 62, 0.8)` with `rgba(255, 255, 255, 0.08)` borders
- Text: `#888` for labels, `#666` for values
- Disc shadow: `0 8px 32px rgba(0,0,0,0.4)` with `1px rgba(255,255,255,0.05)` ring
- Slider tracks: `36px` tall, fully rounded (`border-radius: 18px`), inset shadow
- Slider thumbs: `28px` white circles with drop shadow
- Toggle: gradient `#e94560 → #c73550` with `box-shadow: 0 2px 12px rgba(233, 69, 96, 0.3)`

## Backend Changes

Two new daemon actions, following the same config-driven pattern as `set-color-main`:

### New action: `set-brightness-main`

- Read `live_brightness` (0-100) from config
- Map to ZCL level: `(percentage * 254 / 100) as u8`
- Send `LightCommand::SetBrightness` to main target
- Update `DaemonState::current_brightness`

### New action: `set-colortemp-main`

- Read `live_mirek` (153-500) from config
- Send `LightCommand::SetColorTemperature` to main target
- Update `DaemonState::current_mirek`

### Config schema additions

Two new fields in config.json:
- `live_brightness`: u8 (0-100), persisted alongside existing `live_color_hex`
- `live_mirek`: u16 (153-500), persisted alongside existing `live_color_hex`

### plugin.toml additions

Register `set-brightness-main` and `set-colortemp-main` in `[runtime.actions]`.

## Scope Exclusions

- Preset editor color picker is not changed (stays as hex text input)
- No config migration — new fields default to sensible values if missing (brightness: 100, mirek: 300)
- Touch gesture refinements (pinch-to-zoom, multi-touch) are out of scope
