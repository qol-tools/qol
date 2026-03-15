import { html } from '../../lib/html.js';
import { DevViewInner } from './index.js';

export function DevView() {
    return html`<${DevViewInner} />`;
}
