/** Keep model shortcuts out of text editing, IME and modal interactions. */
export function listenViewportKeydown(handler: (event: KeyboardEvent) => void): () => void {
  const listener = (event: KeyboardEvent) => {
    if (event.defaultPrevented || event.isComposing || event.keyCode === 229) return;
    const target = event.target;
    if (target instanceof Element && target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])')) return;
    if (document.querySelector('[aria-modal="true"]')) return;
    handler(event);
  };
  window.addEventListener('keydown', listener);
  return () => window.removeEventListener('keydown', listener);
}
