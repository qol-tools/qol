const EDITOR_SUFFIX = '-editor';

export function editorParentViewId(viewId) {
    if (!viewId?.endsWith(EDITOR_SUFFIX)) return null;
    return viewId.slice(0, -EDITOR_SUFFIX.length);
}

export function resolveViewKeyboard(viewId, getViewKeyboard, anchorPageId = null) {
    if (anchorPageId && anchorPageId !== viewId) {
        const anchored = getViewKeyboard(anchorPageId);
        if (anchored) return anchored;
        const anchorParent = editorParentViewId(anchorPageId);
        if (anchorParent) {
            const fromAnchorParent = getViewKeyboard(anchorParent);
            if (fromAnchorParent) return fromAnchorParent;
        }
    }
    const direct = getViewKeyboard(viewId);
    if (direct) return direct;
    const parentId = editorParentViewId(viewId);
    return parentId ? getViewKeyboard(parentId) : null;
}
