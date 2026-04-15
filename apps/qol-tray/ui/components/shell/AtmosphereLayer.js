import { html } from '../../lib/html.js';
import { useEffect, useState } from 'preact/hooks';
import { resolvePresetClass } from '../../lib/atmosphere-presets.js';

export function AtmosphereLayer({ navigation }) {
    const [, setTick] = useState(0);
    useEffect(() => {
        if (!navigation?.subscribeAnchor) return undefined;
        return navigation.subscribeAnchor(() => setTick((t) => t + 1));
    }, [navigation]);

    const traits = navigation?.getCurrentTraits?.() || {};
    const atmosphere = traits.atmosphere;
    const presetClass = resolvePresetClass(atmosphere);
    const customBg = typeof atmosphere?.background === 'string' ? atmosphere.background : null;
    if (!presetClass && !customBg) return null;

    const classes = ['atmosphere-layer', 'active'];
    if (presetClass) classes.push(presetClass);
    const style = customBg ? { background: customBg } : undefined;

    return html`<div class=${classes.join(' ')} style=${style} aria-hidden="true"></div>`;
}
