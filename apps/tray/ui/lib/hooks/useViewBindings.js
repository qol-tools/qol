import { VIEW_BINDING_DEFAULTS } from '../view-bindings.js';

export function useViewBindings(viewId) {
    return VIEW_BINDING_DEFAULTS[viewId] || [];
}
