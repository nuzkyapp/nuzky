import { useState } from "react";
import { AlertCircle } from "lucide-react";
import { api, errorText, type WebPage } from "../lib/api";
import { copyText } from "../lib/clipboard";
import { useEditor } from "../lib/store";

/**
 * Opens the page in the browser. When the browser does not open, `failed` gets the page's address; without it, a toast
 * names the page and offers to copy it. A dialog passes `failed` and shows `BrowserFailed`, since toasts sit under dialogs.
 */
export function openWebPage(page: WebPage, failed: (url: string) => void = toastFailure) {
  api.openWebPage(page).catch(failed);
}

const failure = (url: string) => `Couldn't open your browser. Go to ${url.replace("https://", "")} in it.`;

function toastFailure(url: string) {
  const { toast } = useEditor.getState();
  toast({
    kind: "error",
    text: failure(url),
    action: {
      label: "Copy link",
      run: () =>
        void copyText(url, "The clipboard is not available here.").then(
          () => toast({ kind: "success", text: "Link copied. Paste it into your browser." }),
          (e) => toast({ kind: "error", text: errorText(e) }),
        ),
    },
  });
}

/** What `openWebPage` says in a toast, in place, with the address selectable and Copy link. */
export function BrowserFailed({ url }: { url: string }) {
  const [copied, setCopied] = useState<boolean | null>(null);
  return (
    <p className="flex items-start gap-1.5 text-[12px] leading-[18px] text-fg" role="alert">
      <AlertCircle size={14} className="mt-0.5 shrink-0 text-danger" />
      <span className="min-w-0 select-text break-words">
        {failure(url)}{" "}
        <button type="button" data-copy-link onClick={() => void copyText(url, "").then(() => setCopied(true), () => setCopied(false))} className="font-medium text-accent hover:underline">
          {copied ? "Copied" : copied === false ? "Couldn't copy. Select the address." : "Copy link"}
        </button>
      </span>
    </p>
  );
}
