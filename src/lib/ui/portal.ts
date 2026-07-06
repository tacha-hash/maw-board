/** Teleport an element to document.body — replaces @rgossiaux/svelte-headlessui's
 * Portal (dead on Svelte 5: imports removed svelte/internal APIs). */
export function portal(node: HTMLElement) {
  document.body.appendChild(node);
  return {
    destroy() {
      node.remove();
    },
  };
}
