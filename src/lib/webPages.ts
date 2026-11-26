import { api, errorText, type WebPage } from "./api";
import { copyText } from "./clipboard";
import { useEditor } from "./store";

/** Opens the page in the browser; when the browser does not open, a toast offers its address to copy. */
export function openWebPage(page: WebPage) {
  const { toast } = useEditor.getState();
  api.openWebPage(page).catch((url: string) =>
    toast({
      kind: "error",
      text: "Couldn't open your browser. Copy the link and open it there.",
      action: {
        label: "Copy link",
        run: () =>
          void copyText(url, `The clipboard is not available. Go to ${url} in your browser.`).then(
            () => toast({ kind: "success", text: "Link copied. Paste it into your browser." }),
            (e) => toast({ kind: "error", text: errorText(e) }),
          ),
      },
    }),
  );
}
