// View keyboard handler resolution.
//
// During a dive into an editor sub-page (hotkeys-editor, shortcuts-editor,
// task-runner-editor), `activeViewId` stays on the parent view because dive
// doesn't switch top-level views — only the navigation anchor moves. The
// resolver therefore tries the anchor's pageId first (e.g. 'hotkeys-editor'),
// then the active view id, then the parent of an editor sub-page. This lets
// editor sub-pages register their own keyboard handler while still falling
// back to the parent's handler when the sub-page hasn't registered.

const EDITOR_SUFFIX = '-editor';

export function parentViewIdFor(viewId) {
    if (!viewId?.endsWith(EDITOR_SUFFIX)) return null;
    return viewId.slice(0, -EDITOR_SUFFIX.length);
}

export function resolveViewKeyboard(viewId, getViewKeyboard, anchorPageId = null) {
    if (anchorPageId && anchorPageId !== viewId) {
        const anchored = getViewKeyboard(anchorPageId);
        if (anchored) return anchored;
        const anchorParent = parentViewIdFor(anchorPageId);
        if (anchorParent) {
            const fromAnchorParent = getViewKeyboard(anchorParent);
            if (fromAnchorParent) return fromAnchorParent;
        }
    }
    const direct = getViewKeyboard(viewId);
    if (direct) return direct;
    const parentId = parentViewIdFor(viewId);
    return parentId ? getViewKeyboard(parentId) : null;
}
