/**
 * Short-lived layers fade out instead of vanishing. When React removes an element marked `data-exit` (a menu, a dialog,
 * a toast), a lifeless copy of it stays where it was for the fade: it takes no pointer, focus or screen reader, and
 * whatever the layer started goes on at once, as nothing waits for the copy. A menu marked `data-chose` blinks its
 * `data-chosen` row first, as macOS does. `data-exit="float"` copies an element that sat in a stack to the place its
 * `data-exit-rect` (left, top and width in the window) gives, so the rest of the stack can move up at once. A `<dialog>`
 * that stays mounted fades out when it closes, so it keeps what it showed until then.
 */
const ENTRY = ["menu-in", "toast-in", "dialog-in", "scrim-in"];
/** The longest a copy lives: the blink and the fade (index.css), with a frame to spare. */
const LIFE_MS = 50 + 150 + 30;

const reduced = matchMedia("(prefers-reduced-motion: reduce)");

/** Marks `row` as the item chosen in its menu, so the menu's copy blinks it before it fades. */
export function chose(row: Element | null | undefined) {
  row?.setAttribute("data-chosen", "");
  row?.closest("[data-exit]")?.setAttribute("data-chose", "");
}

/** Layers that already left a copy, as a dialog closed and then removed. */
const left = new WeakSet<Node>();

function leave(node: HTMLElement, parent: Node, next: Node | null) {
  if (left.has(node)) return;
  left.add(node);
  const copy = node.cloneNode(true) as HTMLElement;
  // A clone's canvases start blank, as the YouTube dialog's video would be.
  const canvases = copy.querySelectorAll("canvas");
  node.querySelectorAll("canvas").forEach((source, i) => {
    try {
      canvases[i].getContext("2d")?.drawImage(source, 0, 0);
    } catch {
      // A canvas that cannot be read stays blank for the fade.
    }
  });
  // Nothing that finds the live layer, such as a check for an open dialog or menu, finds the copy.
  for (const el of [copy, ...copy.querySelectorAll<HTMLElement>("*")]) {
    for (const { name } of [...el.attributes])
      if (["id", "role", "tabindex", "title", "name", "open"].includes(name) || name.startsWith("aria-") || (name.startsWith("data-") && name !== "data-chosen")) el.removeAttribute(name);
    el.classList.remove(...ENTRY);
  }
  copy.inert = true;
  copy.setAttribute("aria-hidden", "true");
  let ghost = copy;
  if (node.dataset.exit === "float") {
    const [left, top, width] = (node.dataset.exitRect ?? "").split(" ").map(Number);
    // Under the stack (z-90), so the ones sliding into its place pass over it rather than under its blur.
    Object.assign(copy.style, { position: "fixed", left: `${left}px`, top: `${top}px`, width: `${width}px`, margin: "0", zIndex: "89" });
    parent = document.body;
    next = null;
  } else if (node instanceof HTMLDialogElement) {
    // A modal dialog's backdrop belongs to the top layer, which a copy cannot join, so the copy brings its own.
    copy.style.display = "block";
    ghost = document.createElement("div");
    Object.assign(ghost.style, { position: "fixed", inset: "0", zIndex: "100", background: "rgb(0 0 0 / 0.45)" });
    ghost.inert = true;
    ghost.append(copy);
    parent = document.body;
    next = null;
  }
  ghost.classList.add("exit-ghost");
  if (node.hasAttribute("data-chose")) ghost.setAttribute("data-chose", "");
  parent.insertBefore(ghost, next?.parentNode === parent ? next : null);
  window.setTimeout(() => ghost.remove(), LIFE_MS);
}

new MutationObserver((records) => {
  if (reduced.matches) return;
  for (const record of records) {
    const target = record.target;
    if (record.type === "attributes") {
      if (!(target instanceof HTMLDialogElement) || !target.hasAttribute("data-exit")) continue;
      // Opened again, it fades again when it next closes.
      if (target.open) left.delete(target);
      else if (record.oldValue !== null && target.parentNode) leave(target, target.parentNode, target.nextSibling);
      continue;
    }
    for (const node of record.removedNodes) if (node instanceof HTMLElement && node.hasAttribute("data-exit") && target.isConnected) leave(node, target, record.nextSibling);
  }
}).observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["open"], attributeOldValue: true });
