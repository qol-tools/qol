import { html } from '../lib/html.js';
import { useStoreController } from './store/use-controller.js';
import { StoreLayout } from './store/layout.js';

export function StoreView() {
    const ctrl = useStoreController();
    StoreView.handleKey = ctrl.handleKey;
    StoreView.isBlocking = () => false;
    return html`<${StoreLayout} ctrl=${ctrl} />`;
}
