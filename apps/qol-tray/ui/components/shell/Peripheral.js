import { html } from '../../lib/html.js';
import { useRef } from 'preact/hooks';
import { useOverlayHide } from '../../lib/hooks/useIdleHide.js';

const EDGE_CLASS = {
    br: 'peripheral-edge-br',
    bl: 'peripheral-edge-bl',
    top: 'peripheral-edge-top',
    tl: 'peripheral-edge-tl',
    tr: 'peripheral-edge-tr',
};

export function Peripheral({
    camera,
    navigation,
    alwaysVisible = false,
    edge,
    occludeSelector,
    as: Tag = 'div',
    className,
    elementRef,
    children,
    ...rest
}) {
    const localRef = useRef(null);
    const ref = elementRef || localRef;
    useOverlayHide({ targetRef: ref, camera, navigation, alwaysVisible, occludeSelector });
    const classes = ['peripheral-edge-dock', EDGE_CLASS[edge], className].filter(Boolean).join(' ');
    return html`<${Tag} ref=${ref} class=${classes} ...${rest}>${children}<//>`;
}
