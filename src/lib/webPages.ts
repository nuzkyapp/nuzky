import { api, errorText, type WebPage } from "./api";
import { copyText } from "./clipboard";
import { useEditor } from "./store";

/** Opens the page in the browser; when the browser does not open, a toast names the page and offers to copy its address. */
export function openWebPage(page: WebPage) {
  const { toast } = useEditor.getState();
  api.openWebPage(page).catch((url: string) =>
    toast({
      kind: "error",
      text: `Couldn't open your browser. Go to ${url.replace("https://", "")} in it.`,
      action: {
        label: "Copy link",
        run: () =>
          void copyText(url, "The clipboard is not available here.").then(
            () => toast({ kind: "success", text: "Link copied. Paste it into your browser." }),
            (e) => toast({ kind: "error", text: errorText(e) }),
          ),
      },
    }),
  );
}
