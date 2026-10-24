import { type RefObject, useEffect, useState } from "react";

// Whether the element is on screen in a visible tab, and whether the visitor has asked for reduced motion.
// All three are followed live, so a demo stops in a hidden tab and the moment reduced motion is switched on.
export function useView(ref: RefObject<Element | null>, threshold = 0) {
  const [view, setView] = useState({ shown: false, still: false });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const still = window.matchMedia("(prefers-reduced-motion: reduce)");
    let onScreen = false;
    const update = () => setView({ shown: onScreen && !document.hidden, still: still.matches });
    const io = new IntersectionObserver(
      ([e]) => {
        onScreen = e.isIntersecting;
        update();
      },
      { threshold },
    );
    io.observe(el);
    document.addEventListener("visibilitychange", update);
    still.addEventListener("change", update);
    return () => {
      io.disconnect();
      document.removeEventListener("visibilitychange", update);
      still.removeEventListener("change", update);
    };
  }, [ref, threshold]);
  return view;
}

// A demo plays only while its section is on screen, the tab is visible and the visitor has not
// asked for reduced motion.
export function usePlaying(ref: RefObject<Element | null>, threshold = 0) {
  const { shown, still } = useView(ref, threshold);
  return shown && !still;
}
