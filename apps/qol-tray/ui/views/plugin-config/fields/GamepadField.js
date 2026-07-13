import { html } from '../../../lib/html.js';
import { useCallback, useMemo, useRef, useState } from 'preact/hooks';
import { GamepadIllustration } from '../../../assets/gamepad-illustration.js';
import { CustomSelect } from '../../../lib/components/CustomSelect.js';
import { usePluginConfigContext } from '../context.js';
import { fieldSurfaceAttrs } from '../field-map.js';
import {
    activeInputs,
    connectionPresentation,
    formatSigned,
    formatValue,
} from './gamepad-model.js';
import {
    controllerProfile,
    unmappedProfileButtons,
} from './gamepad-profiles.js';
import { useGamepadMonitor } from './useGamepadMonitor.js';

export function GamepadField({ field }) {
    const ctx = usePluginConfigContext();
    const containerRef = useRef(null);
    const [preference, setPreference] = useState('auto');
    const queryDef = ctx.runtime?.query?.[field.query];
    const monitor = useGamepadMonitor(containerRef, preference, {
        pluginId: ctx.pluginId,
        queryName: field.query,
        intervalMs: queryDef?.poll_interval_ms,
    });
    const onSelect = useCallback(() => ctx.setSelectedFieldId(field.id), [ctx, field.id]);
    const selector = useMemo(() => gamepadSelector(monitor.gamepads), [monitor.gamepads]);
    const effectivePreference = selector.options.includes(preference) ? preference : 'auto';

    return html`
        <div ref=${containerRef}
            ...${fieldSurfaceAttrs(field, ctx, 'field-group field-gamepad')}
            onMouseDown=${onSelect}
            onFocus=${onSelect}>
            <div class="gamepad-heading">
                <div>
                    <div class="gamepad-label">${field.label}</div>
                    ${field.description && html`<div class="field-help">${field.description}</div>`}
                </div>
                <div class="status-chip status-chip--${statusTone(monitor.status)}">
                    ${statusLabel(monitor.status)}
                </div>
            </div>
            ${selector.options.length > 2 && html`
                <div class="gamepad-selector">
                    <span>Controller source</span>
                    <${CustomSelect}
                        value=${effectivePreference}
                        options=${selector.options}
                        labels=${selector.labels}
                        onChange=${setPreference} />
                </div>
            `}
            ${monitor.selected
                ? html`<${GamepadTester} snapshot=${monitor.selected} />`
                : html`<${GamepadWaiting} status=${monitor.status} message=${monitor.message} />`
            }
        </div>
    `;
}

function GamepadTester({ snapshot }) {
    const active = activeInputs(snapshot);
    const profile = controllerProfile(snapshot);
    const unmappedButtons = unmappedProfileButtons(snapshot, profile);
    return html`
        <div class="gamepad-tester">
            <div class="gamepad-device-row">
                <div class="gamepad-device-name" title=${snapshot.id}>${snapshot.id}</div>
                <div class="gamepad-device-meta">
                    <${ConnectionIndicator} connection=${snapshot.connection} />
                    <span>${snapshot.mapping === 'standard' ? 'Standard mapping' : 'Raw mapping'}</span>
                    <span>${snapshot.buttons.length} buttons</span>
                    <span>${snapshot.axes.length} axes</span>
                    <span>${profile.label}</span>
                    ${snapshot.mappingProfile && html`<span>Corrected layout</span>`}
                    ${snapshot.nativeInput && html`<span>Native stick clicks</span>`}
                    ${snapshot.haptics && html`<span>Haptics</span>`}
                </div>
            </div>
            ${snapshot.mappingProfile && !snapshot.nativeInput && html`
                <div class="gamepad-compatibility-note">
                    Button layout corrected. L3/R3 still depend on native controller access.
                </div>
            `}
            ${profile.deviceNote && html`
                <div class="gamepad-device-control-note">
                    <strong>On-controller controls</strong>
                    <span>${profile.deviceNote}</span>
                </div>
            `}
            <${GamepadDiagram} snapshot=${snapshot} profile=${profile} unmappedButtons=${unmappedButtons} />
            <${GamepadReadout} snapshot=${snapshot} active=${active} unmappedButtons=${unmappedButtons} />
        </div>
    `;
}

function ConnectionIndicator({ connection }) {
    const signal = connectionPresentation(connection);
    if (!signal) return null;
    const hasBars = signal.level !== null;
    return html`
        <div class="gamepad-connection gamepad-connection--${signal.tone}"
            role="img" aria-label=${signal.label} title=${signal.label}>
            ${hasBars && html`
                <span class="gamepad-signal-bars" aria-hidden="true">
                    ${[1, 2, 3, 4].map(level => html`
                        <i key=${level} data-active=${level <= signal.level}></i>
                    `)}
                </span>
            `}
            <span>${signal.transport}</span>
            <strong>${signal.detail}</strong>
            ${signal.signalDbm !== null && html`<output>${signal.signalDbm} dBm</output>`}
        </div>
    `;
}

function GamepadWaiting({ status, message }) {
    return html`
        <div class="gamepad-waiting" data-status=${status}>
            <div class="gamepad-waiting-orbit" aria-hidden="true">
                <span></span><span></span><span></span>
            </div>
            <strong>${status === 'waiting' ? 'Wake a controller' : 'Controller input unavailable'}</strong>
            <span>${message}</span>
        </div>
    `;
}

function GamepadDiagram({ snapshot, profile, unmappedButtons }) {
    return html`
        <div class="gamepad-diagram">
            <${GamepadIllustration}
                buttons=${snapshot.buttons}
                axes=${snapshot.axes}
                profile=${profile}
                unmappedButtons=${unmappedButtons}
                active=${activeInputs(snapshot).length > 0} />
        </div>
    `;
}

function GamepadReadout({ snapshot, active, unmappedButtons }) {
    const unmappedIndices = new Set(unmappedButtons.map(button => button.index));
    return html`
        <div class="gamepad-readout">
            <div class="gamepad-active-inputs">
                <span>Active inputs</span>
                <strong>${active.length > 0 ? active.join(' · ') : 'Waiting for movement'}</strong>
            </div>
            <div class="gamepad-axis-grid">
                ${snapshot.axes.map(axis => html`
                    <div class="gamepad-axis" key=${axis.index}>
                        <span>${axis.name}</span>
                        <div><i style=${`--axis-value:${axis.value}`}></i></div>
                        <output>${formatSigned(axis.value)}</output>
                    </div>
                `)}
            </div>
            ${unmappedButtons.length > 0 && html`
                <div class="gamepad-unmapped-note">
                    <strong>${unmappedButtons.length} auto-discovered input${unmappedButtons.length === 1 ? '' : 's'}</strong>
                    <span>No profile slot exists yet, so generated markers were added below the controller.</span>
                </div>
            `}
            <div class="gamepad-button-grid" role="list" aria-label="Button values">
                ${snapshot.buttons.map(button => html`
                    <div class="gamepad-button-chip" role="listitem" key=${button.index}
                        data-unmapped=${unmappedIndices.has(button.index)}
                        data-active=${button.pressed || button.value > 0.05}>
                        <span>${button.name}</span>
                        <output>${formatValue(button.value)}</output>
                    </div>
                `)}
            </div>
        </div>
    `;
}

function gamepadSelector(gamepads) {
    const options = ['auto', ...gamepads.map(gamepad => String(gamepad.index))];
    const labels = { auto: 'Auto · most recently active' };
    for (const gamepad of gamepads) {
        labels[String(gamepad.index)] = `#${gamepad.index + 1} · ${gamepad.id}`;
    }
    return { options, labels };
}

function statusTone(status) {
    if (status === 'ready') return 'success';
    if (status === 'waiting') return 'warning';
    if (status === 'blocked') return 'danger';
    return 'neutral';
}

function statusLabel(status) {
    if (status === 'ready') return 'Live';
    if (status === 'waiting') return 'Waiting';
    if (status === 'blocked') return 'Blocked';
    return 'Unsupported';
}
