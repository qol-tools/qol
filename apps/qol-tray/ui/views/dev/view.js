import { html } from '../../lib/html.js';
import { DevViewInner } from './index.js';

export function DevView() {
    DevView.handleKey = DevViewInner.handleKey;
    DevView.isBlocking = DevViewInner.isBlocking || (() => false);
    return html`<${DevViewInner} />`;
}
