export function isKeyboardMode() {
    return document.querySelector('.app-container')?.dataset.inputMode !== 'mouse';
}
