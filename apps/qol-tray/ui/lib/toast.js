export function toast(type, message) {
    window.dispatchEvent(new CustomEvent('app-toast', { detail: { type, message } }));
}
