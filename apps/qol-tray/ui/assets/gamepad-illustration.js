import { html } from '../lib/html.js';
import { GAMEPAD_BODY_PATH } from './gamepad-geometry.js';

const EMPTY_BUTTON = Object.freeze({ pressed: false, touched: false, value: 0 });

export function GamepadIllustration({
    buttons = [],
    axes = [],
    active = false,
    profile,
    unmappedButtons = [],
}) {
    const mappedButtons = profile.standardMapping ? buttons : [];
    const mappedAxes = profile.standardMapping ? axes : [];
    const layout = profile.layout;
    const auxiliaryRows = Math.ceil(Math.min(unmappedButtons.length, 16) / 8);
    const viewHeight = auxiliaryRows > 0 ? 515 + (auxiliaryRows * 42) : 500;
    return html`
        <svg class="gamepad-vector" viewBox=${`0 0 800 ${viewHeight}`} role="img"
            aria-label=${profile.ariaLabel}>
            <${GamepadBody} />
            <${TopControls} buttons=${mappedButtons} profile=${profile} />
            <rect class="gamepad-vector-port" data-active=${active} x="380" y="91" width="40" height="6" rx="3" />
            <${Stick} x=${layout.leftStick.x} y=${layout.leftStick.y} label="L3"
                axisX=${axisAt(mappedAxes, 0)} axisY=${axisAt(mappedAxes, 1)}
                pressed=${buttonAt(mappedButtons, 10).pressed} />
            <g transform=${`translate(${layout.dpad.x - 250} ${layout.dpad.y - 310})`}>
                <${Dpad} buttons=${mappedButtons} />
            </g>
            <${CenterControls} buttons=${mappedButtons} profile=${profile} />
            <${FaceButtons} buttons=${mappedButtons} profile=${profile} />
            <${Stick} x=${layout.rightStick.x} y=${layout.rightStick.y} label="R3"
                axisX=${axisAt(mappedAxes, 2)} axisY=${axisAt(mappedAxes, 3)}
                pressed=${buttonAt(mappedButtons, 11).pressed} />
            <${AuxiliaryControls} buttons=${unmappedButtons} />
        </svg>
    `;
}

function TopControls({ buttons, profile }) {
    const triggers = profile.triggerLabels;
    const shoulders = profile.shoulderLabels;
    return html`
        <g class="gamepad-vector-top-controls">
            <${Trigger} side="left" label=${triggers[0]} button=${buttonAt(buttons, 6)} />
            <${Trigger} side="right" label=${triggers[1]} button=${buttonAt(buttons, 7)} />
            <${Shoulder} side="left" label=${shoulders[0]} button=${buttonAt(buttons, 4)} />
            <${Shoulder} side="right" label=${shoulders[1]} button=${buttonAt(buttons, 5)} />
        </g>
    `;
}

function GamepadBody() {
    return html`
        <g class="gamepad-vector-bodywork">
            <path class="gamepad-vector-body" d=${GAMEPAD_BODY_PATH} />
            <path class="gamepad-vector-crown" d="M270 82 C308 69 351 63 400 63 C449 63 492 69 530 82 C499 104 456 115 400 115 C344 115 301 104 270 82 Z" />
            <path class="gamepad-vector-grip" d="M77 244 L43 390 C32 438 58 479 101 487 C132 493 158 480 180 452 L246 370 C218 342 189 326 153 317 C118 308 94 284 77 244 Z" />
            <path class="gamepad-vector-grip" d="M723 244 L757 390 C768 438 742 479 699 487 C668 493 642 480 620 452 L554 370 C582 342 611 326 647 317 C682 308 706 284 723 244 Z" />
            <path class="gamepad-vector-seam" d="M94 291 C139 316 199 335 246 370" />
            <path class="gamepad-vector-seam" d="M706 291 C661 316 601 335 554 370" />
            <path class="gamepad-vector-seam" d="M270 82 C305 100 348 108 400 108 C452 108 495 100 530 82" />
        </g>
    `;
}

function Trigger({ side, label, button }) {
    const left = side === 'left';
    const path = left
        ? 'M197 84 C201 61 211 47 231 39 C249 37 269 39 288 43 L282 85 Z'
        : 'M603 84 C599 61 589 47 569 39 C551 37 531 39 512 43 L518 85 Z';
    const x = left ? 241 : 559;
    return html`
        <g class="gamepad-vector-input gamepad-vector-trigger" data-active=${button.value > 0.05}>
            <path class="gamepad-vector-control" d=${path} />
            <path class="gamepad-vector-pressure" d=${path} style=${`--input-value:${button.value}`} />
            <text x=${x} y="61" text-anchor="middle">${label}</text>
            <text class="gamepad-vector-value" x=${x} y="75" text-anchor="middle">${formatValue(button.value)}</text>
        </g>
    `;
}

function Shoulder({ side, label, button }) {
    const left = side === 'left';
    const path = left
        ? 'M142 119 C169 88 214 76 289 81 L280 116 C225 109 183 115 154 133 Z'
        : 'M658 119 C631 88 586 76 511 81 L520 116 C575 109 617 115 646 133 Z';
    return html`
        <g class="gamepad-vector-input gamepad-vector-shoulder" data-active=${button.pressed}>
            <path class="gamepad-vector-control" d=${path} />
            <text x=${left ? 215 : 585} y="107" text-anchor="middle">${label}</text>
        </g>
    `;
}

function Stick({ x, y, label, axisX, axisY, pressed }) {
    const moving = Math.abs(axisX) > 0.08 || Math.abs(axisY) > 0.08;
    const offsetX = (axisX * 18).toFixed(2);
    const offsetY = (axisY * 18).toFixed(2);
    return html`
        <g class="gamepad-vector-stick" data-active=${pressed || moving}>
            <circle class="gamepad-vector-stick-gate" cx=${x} cy=${y} r="57" />
            <path class="gamepad-vector-stick-crosshair" d=${`M${x - 43} ${y} H${x + 43} M${x} ${y - 43} V${y + 43}`} />
            <g class="gamepad-vector-stick-knob" transform=${`translate(${offsetX} ${offsetY})`}>
                <circle cx=${x} cy=${y} r="31" />
                <circle class="gamepad-vector-stick-ring" cx=${x} cy=${y} r="23" />
                <text x=${x} y=${y} text-anchor="middle">${label}</text>
            </g>
        </g>
    `;
}

function Dpad({ buttons }) {
    return html`
        <g class="gamepad-vector-dpad">
            <circle class="gamepad-vector-control-well" cx="250" cy="310" r="58" />
            <path class="gamepad-vector-dpad-base" d="M236 263 Q236 255 244 255 H256 Q264 255 264 263 V296 H297 Q305 296 305 304 V316 Q305 324 297 324 H264 V357 Q264 365 256 365 H244 Q236 365 236 357 V324 H203 Q195 324 195 316 V304 Q195 296 203 296 H236 Z" />
            <${DpadDirection} direction="up" button=${buttonAt(buttons, 12)} />
            <${DpadDirection} direction="right" button=${buttonAt(buttons, 15)} />
            <${DpadDirection} direction="down" button=${buttonAt(buttons, 13)} />
            <${DpadDirection} direction="left" button=${buttonAt(buttons, 14)} />
            <circle class="gamepad-vector-dpad-pivot" cx="250" cy="310" r="14" />
        </g>
    `;
}

function DpadDirection({ direction, button }) {
    const paths = {
        up: 'M236 296 V263 Q236 255 244 255 H256 Q264 255 264 263 V296 L250 310 Z',
        right: 'M264 296 H297 Q305 296 305 304 V316 Q305 324 297 324 H264 L250 310 Z',
        down: 'M264 324 V357 Q264 365 256 365 H244 Q236 365 236 357 V324 L250 310 Z',
        left: 'M236 324 H203 Q195 324 195 316 V304 Q195 296 203 296 H236 L250 310 Z',
    };
    const marks = {
        up: [250, 275, '▲'],
        right: [285, 310, '▶'],
        down: [250, 345, '▼'],
        left: [215, 310, '◀'],
    };
    const [x, y, mark] = marks[direction];
    return html`
        <g class="gamepad-vector-dpad-input" data-active=${button.pressed}>
            <path d=${paths[direction]} />
            <text x=${x} y=${y} text-anchor="middle">${mark}</text>
        </g>
    `;
}

function CenterControls({ buttons, profile }) {
    if (profile?.centerVariant === 'gulikit') {
        return html`<${GulikitCenterControls} buttons=${buttons} controls=${profile.deviceControls} />`;
    }
    if (profile?.centerVariant === 'playstation') {
        return html`<${PlayStationCenterControls} buttons=${buttons} />`;
    }
    if (profile?.centerVariant === 'switch') {
        return html`<${SwitchCenterControls} buttons=${buttons} />`;
    }
    const home = buttonAt(buttons, 16);
    const view = buttonAt(buttons, 8);
    const menu = buttonAt(buttons, 9);
    return html`
        <g class="gamepad-vector-center-controls">
            <g class="gamepad-vector-input gamepad-vector-home" data-active=${home.pressed}>
                <circle class="gamepad-vector-control" cx="400" cy="164" r="23" />
                <text x="400" y="164" text-anchor="middle">Q</text>
            </g>
            <g class="gamepad-vector-input gamepad-vector-center-button" data-active=${view.pressed}>
                <rect class="gamepad-vector-control" x="332" y="202" width="44" height="26" rx="13" />
                <rect x="346" y="210" width="9" height="7" rx="1" />
                <rect x="351" y="213" width="9" height="7" rx="1" />
            </g>
            <g class="gamepad-vector-input gamepad-vector-center-button" data-active=${menu.pressed}>
                <rect class="gamepad-vector-control" x="424" y="202" width="44" height="26" rx="13" />
                <path d="M439 210 H453 M439 215 H453 M439 220 H453" />
            </g>
        </g>
    `;
}

function GulikitCenterControls({ buttons, controls }) {
    return html`
        <g class="gamepad-vector-center-controls gamepad-vector-center-controls--gulikit">
            <${RoundCenterButton} button=${buttonAt(buttons, 8)} x=${338} y=${164} label="−" />
            <${RoundCenterButton} button=${buttonAt(buttons, 16)} x=${400} y=${164} label="G" home=${true} />
            <${RoundCenterButton} button=${buttonAt(buttons, 9)} x=${462} y=${164} label="+" />
            ${(controls || []).map(control => html`<${DeviceControl} key=${control.id} control=${control} />`)}
        </g>
    `;
}

function PlayStationCenterControls({ buttons }) {
    return html`
        <g class="gamepad-vector-center-controls gamepad-vector-center-controls--playstation">
            <${PillCenterButton} button=${buttonAt(buttons, 8)} x=${334} y=${178} label="SHARE" />
            <${PillCenterButton} button=${buttonAt(buttons, 9)} x=${466} y=${178} label="OPTIONS" />
            <${RoundCenterButton} button=${buttonAt(buttons, 16)} x=${400} y=${248} label="PS" home=${true} />
        </g>
    `;
}

function SwitchCenterControls({ buttons }) {
    return html`
        <g class="gamepad-vector-center-controls gamepad-vector-center-controls--switch">
            <${RoundCenterButton} button=${buttonAt(buttons, 8)} x=${350} y=${181} label="−" />
            <${RoundCenterButton} button=${buttonAt(buttons, 9)} x=${450} y=${181} label="+" />
            <${RoundCenterButton} button=${buttonAt(buttons, 16)} x=${400} y=${236} label="⌂" home=${true} />
        </g>
    `;
}

function RoundCenterButton({ button, x, y, label, home = false }) {
    return html`
        <g class=${`gamepad-vector-input gamepad-vector-round-button${home ? ' gamepad-vector-home' : ''}`}
            data-active=${button.pressed}>
            <circle class="gamepad-vector-control" cx=${x} cy=${y} r=${home ? 23 : 16} />
            <text x=${x} y=${y} text-anchor="middle">${label}</text>
        </g>
    `;
}

function PillCenterButton({ button, x, y, label }) {
    return html`
        <g class="gamepad-vector-input gamepad-vector-pill-button" data-active=${button.pressed}>
            <rect class="gamepad-vector-control" x=${x - 31} y=${y - 12} width="62" height="24" rx="12" />
            <text x=${x} y=${y} text-anchor="middle">${label}</text>
        </g>
    `;
}

function DeviceControl({ control }) {
    return html`
        <g class="gamepad-vector-device-control" data-device-only="true">
            <title>${control.label}: handled by controller firmware; no PC XInput button event</title>
            <circle class="gamepad-vector-control" cx=${control.x} cy=${control.y} r="15" />
            <text x=${control.x} y=${control.y} text-anchor="middle">${control.shortLabel}</text>
        </g>
    `;
}

function FaceButtons({ buttons, profile }) {
    const center = profile.layout.face;
    const controls = profile.faceButtons;
    return html`
        <g class="gamepad-vector-face-buttons">
            ${controls.map(control => {
                const x = center.x + control.dx;
                const y = center.y + control.dy;
                return html`
                <g key=${control.label} class=${`gamepad-vector-input gamepad-vector-face gamepad-vector-face--${control.tone}`}
                    data-active=${buttonAt(buttons, control.index).pressed}>
                    <circle class="gamepad-vector-control" cx=${x} cy=${y} r="23" />
                    <text x=${x} y=${y} text-anchor="middle">${control.label}</text>
                </g>
            `;})}
        </g>
    `;
}

function AuxiliaryControls({ buttons }) {
    const visible = buttons.slice(0, 16);
    if (visible.length === 0) return null;
    return html`
        <g class="gamepad-vector-auxiliary-controls">
            <text class="gamepad-vector-auxiliary-label" x="88" y="521">AUTO-DISCOVERED</text>
            ${visible.map((button, index) => {
                const column = index % 8;
                const row = Math.floor(index / 8);
                const x = 226 + (column * 65);
                const y = 516 + (row * 42);
                return html`
                    <g key=${button.index} class="gamepad-vector-input gamepad-vector-auxiliary"
                        data-active=${button.pressed || button.value > 0.05}>
                        <circle class="gamepad-vector-control" cx=${x} cy=${y} r="17" />
                        <text x=${x} y=${y} text-anchor="middle">${button.name}</text>
                    </g>
                `;
            })}
            ${buttons.length > visible.length && html`
                <text class="gamepad-vector-auxiliary-overflow" x="754" y="521" text-anchor="end">
                    +${buttons.length - visible.length}
                </text>
            `}
        </g>
    `;
}

function buttonAt(buttons, index) {
    return buttons[index] || EMPTY_BUTTON;
}

function axisAt(axes, index) {
    return axes[index]?.value || 0;
}

function formatValue(value) {
    return Number(value || 0).toFixed(2);
}
