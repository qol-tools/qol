export function createLinkInputState({ state, onNeedsRender }) {
    function showLinkInput() {
        state.showLinkInput = true;
        state.linkError = null;
        onNeedsRender();
    }

    function cancelLink() {
        state.showLinkInput = false;
        state.linkPath = '';
        state.linkError = null;
        onNeedsRender();
    }

    function readLinkPath() {
        if (state.linkPath.trim()) {
            return state.linkPath;
        }

        state.linkError = 'Enter a path';
        onNeedsRender();
        return null;
    }

    function clearLinkInput() {
        state.showLinkInput = false;
        state.linkPath = '';
        state.linkError = null;
    }

    function failLink(message) {
        state.linkError = message;
        onNeedsRender();
    }

    return {
        cancelLink,
        clearLinkInput,
        failLink,
        readLinkPath,
        showLinkInput
    };
}
